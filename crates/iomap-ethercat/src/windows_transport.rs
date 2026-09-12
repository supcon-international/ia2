//! Windows packet transport around EtherCrab's public PDU API.
//!
//! EtherCrab 0.7.1's Windows std pump links wpcap at process startup,
//! panics on capture/send failures, and only checks exit with no frames
//! in flight. Keep its protocol core, but own this stoppable transport.
//! The Npcap DLL is deliberately loaded only when opening a real device.

use ethercrab::{PduRx, PduTx};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::task::{Wake, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

#[cfg(windows)]
mod npcap;

pub(super) type PumpError = Arc<Mutex<Option<String>>>;

/// This is only the packet boundary, not another EtherCAT implementation.
/// Receive must be nonblocking; the slice stays valid until the next call.
trait PacketIo {
    fn send(&mut self, bytes: &[u8]) -> Result<usize, String>;
    fn receive(&mut self) -> Result<Option<&[u8]>, String>;
}

// Keep the storage alive on EVERY thread exit path, including a failed
// open, failed startup handshake, unwinding and a detached driver call.
// Field order is intentional: both PDU handles are destroyed first.
struct PacketHandles {
    tx: PduTx<'static>,
    rx: PduRx<'static>,
    _storage: Arc<dyn Send + Sync>,
}

impl PacketHandles {
    fn run(self, io: &mut impl PacketIo, stop: &AtomicBool) -> Result<(), String> {
        let Self { tx, rx, _storage } = self;
        // The local owner remains alive until run_pump has dropped both
        // handles, including on panic. Do not inline/destructure in the
        // thread closure: capturing the whole owner preserves drop order.
        run_pump(io, tx, rx, stop)
    }
}

pub(super) struct Pump {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    stop_confirmed: Arc<AtomicBool>,
}

#[cfg(windows)]
pub(super) struct Prepared(npcap::Capture);

#[cfg(windows)]
pub(super) fn prepare(nic: &str) -> Result<Prepared, String> {
    npcap::Capture::open(nic).map(Prepared)
}

struct ThreadWake(thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

impl Pump {
    #[cfg(windows)]
    pub(super) fn start(
        prepared: Prepared,
        tx: PduTx<'static>,
        rx: PduRx<'static>,
        dead: Arc<AtomicBool>,
        error: PumpError,
        stop_confirmed: Arc<AtomicBool>,
        storage: Arc<dyn Send + Sync>,
    ) -> Result<Self, String> {
        let handles = PacketHandles {
            tx,
            rx,
            _storage: storage,
        };
        let mut pump = Self::start_owned(move || Ok(prepared.0), handles, dead, error)?;
        pump.stop_confirmed = stop_confirmed;
        Ok(pump)
    }

    #[cfg(test)]
    fn start_with<I: PacketIo, F: FnOnce() -> Result<I, String> + Send + 'static>(
        open: F,
        tx: PduTx<'static>,
        rx: PduRx<'static>,
        dead: Arc<AtomicBool>,
        error: PumpError,
    ) -> Result<Self, String> {
        // Tests using this convenience constructor have static test
        // storage. Ownership-specific tests use start_owned directly.
        Self::start_owned(
            open,
            PacketHandles {
                tx,
                rx,
                _storage: Arc::new(()),
            },
            dead,
            error,
        )
    }

    fn start_owned<I: PacketIo, F: FnOnce() -> Result<I, String> + Send + 'static>(
        open: F,
        handles: PacketHandles,
        dead: Arc<AtomicBool>,
        error: PumpError,
    ) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("ec-npcap".into())
            .spawn(move || {
                struct Exit(Arc<AtomicBool>);
                impl Drop for Exit {
                    fn drop(&mut self) {
                        self.0.store(true, Ordering::Release);
                    }
                }
                let _exit = Exit(dead);
                let mut io = match open() {
                    Ok(io) => io,
                    Err(message) => {
                        let _ = ready_tx.send(Err(message));
                        return;
                    }
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                if let Err(message) = handles.run(&mut io, &worker_stop) {
                    tracing::error!(%message, "EtherCAT Npcap transport failed");
                    *error.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                }
            })
            .map_err(|e| format!("spawn Npcap packet thread: {e}"))?;
        let pump = Self {
            stop,
            thread: Some(worker),
            stop_confirmed: Arc::new(AtomicBool::new(true)),
        };
        ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|e| format!("Npcap startup did not complete: {e}"))??;
        Ok(pump)
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.thread.take() {
            worker.thread().unpark();
            // Nonblocking packet reads plus a bounded batch allow exit
            // even after a cable break or while unrelated traffic floods
            // the NIC. A stuck third-party kernel call cannot be killed
            // safely; report that condition and bound application teardown.
            let deadline = Instant::now() + Duration::from_millis(500);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            if worker.is_finished() {
                if worker.join().is_err() {
                    tracing::error!("EtherCAT Npcap packet thread panicked");
                }
            } else {
                self.stop_confirmed.store(false, Ordering::Release);
                tracing::error!("EtherCAT Npcap packet thread did not stop within 500 ms");
            }
        }
    }
}

fn run_pump(
    io: &mut impl PacketIo,
    mut tx: PduTx<'_>,
    mut rx: PduRx<'_>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    while !stop.load(Ordering::Acquire) && !tx.should_exit() {
        tx.replace_waker(&waker);
        // Bound both directions so continuous traffic cannot starve exit
        // or the other direction. No in-flight counter can wedge shutdown.
        for _ in 0..tx.capacity() {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some(frame) = tx.next_sendable_frame() else {
                break;
            };
            let mut send_error = None;
            let result = frame.send_blocking(|bytes| match io.send(bytes) {
                Ok(sent) => Ok(sent),
                Err(message) => {
                    send_error = Some(message);
                    Err(ethercrab::error::Error::SendFrame)
                }
            });
            if let Err(e) = result {
                return Err(format!(
                    "Npcap frame send failed: {}",
                    send_error.unwrap_or_else(|| format!("{e:?}"))
                ));
            }
        }
        for _ in 0..64 {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let Some(frame) = io.receive()? else {
                break;
            };
            // Filter before parsing, including short non-Ethernet traffic.
            if frame.get(12..14) != Some(&[0x88, 0xa4]) {
                continue;
            }
            rx.receive_frame(frame)
                .map_err(|e| format!("EtherCAT receive frame: {e:?}"))?;
        }
        // Rust's Windows thread sleep uses a high-resolution waitable
        // timer. This bounds polling cost without a permanently busy CPU.
        // Windows remains soft real-time; actual bus jitter needs hardware
        // qualification. Do not use Windows' coarser park_timeout here.
        thread::sleep(Duration::from_micros(100));
    }
    Ok(())
}

pub(super) fn ethercat_now() -> u64 {
    ethercat_time(SystemTime::now())
}

fn ethercat_time(now: SystemTime) -> u64 {
    // EtherCAT epoch: 2000-01-01. Subtract seconds before converting to ns.
    // The upstream Windows helper subtracts seconds from a ns count.
    now.duration_since(SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800))
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn npcap_name(nic: &str) -> Option<String> {
    let guid = if nic
        .get(..12)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\Device\NPF_"))
    {
        &nic[12..]
    } else {
        nic
    };
    let bytes = guid.as_bytes();
    (bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes[1..37].iter().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                *b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        }))
    .then(|| format!(r"\Device\NPF_{}", guid.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethercrab::{MainDevice, MainDeviceConfig, PduStorage, Timeouts};
    use std::sync::atomic::AtomicUsize;

    struct FakeIo {
        send_result: Result<usize, String>,
        sent: Arc<AtomicUsize>,
        receive_error: bool,
        flood: bool,
    }
    impl PacketIo for FakeIo {
        fn send(&mut self, bytes: &[u8]) -> Result<usize, String> {
            self.sent.fetch_add(1, Ordering::Relaxed);
            self.send_result.clone().map(|n| n.min(bytes.len()))
        }
        fn receive(&mut self) -> Result<Option<&[u8]>, String> {
            if self.receive_error {
                Err("adapter removed".into())
            } else if self.flood {
                Ok(Some(&[0; 7]))
            } else {
                Ok(None)
            }
        }
    }

    fn handles() -> (PduTx<'static>, PduRx<'static>, MainDevice<'static>) {
        let storage = Box::leak(Box::new(PduStorage::<8, 256>::new()));
        let (tx, rx, pdus) = storage.try_split().unwrap();
        (
            tx,
            rx,
            MainDevice::new(pdus, Timeouts::default(), MainDeviceConfig::default()),
        )
    }

    #[test]
    fn guid_selector_is_strict_and_unicode_alias_does_not_panic() {
        let guid = "{12345678-1234-ABCD-9876-123456789ABC}";
        assert_eq!(npcap_name(guid), Some(format!(r"\Device\NPF_{guid}")));
        assert_eq!(
            npcap_name(&format!(r"\device\npf_{guid}")),
            npcap_name(guid)
        );
        for invalid in [
            "abcdefghijk中",
            "以太网适配器网络",
            r"\Device\NPF_Loopback",
            "rpcap://host/eth0",
            "{}",
        ] {
            assert_eq!(npcap_name(invalid), None);
        }
    }

    #[test]
    fn ethercat_epoch_uses_nanoseconds_and_saturates_before_2000() {
        let epoch = SystemTime::UNIX_EPOCH + Duration::from_secs(946_684_800);
        assert_eq!(ethercat_time(epoch), 0);
        assert_eq!(
            // Windows SystemTime represents 100 ns ticks.
            ethercat_time(epoch + Duration::from_nanos(1_000_000_100)),
            1_000_000_100
        );
        assert_eq!(ethercat_time(SystemTime::UNIX_EPOCH), 0);
        assert!(ethercat_now() > 0);
    }

    #[test]
    fn startup_failure_is_returned_and_thread_exits() {
        let (tx, rx, _main) = handles();
        let dead = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let result = Pump::start_with::<FakeIo, _>(
            || Err("Npcap missing".into()),
            tx,
            rx,
            dead.clone(),
            error,
        );
        assert!(result.err().unwrap().contains("Npcap missing"));
        assert!(dead.load(Ordering::Acquire));
    }

    #[test]
    fn receive_failure_is_recorded_and_thread_exits() {
        let (tx, rx, _main) = handles();
        let dead = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let pump = Pump::start_with(
            || {
                Ok(FakeIo {
                    send_result: Ok(usize::MAX),
                    sent: Arc::default(),
                    receive_error: true,
                    flood: false,
                })
            },
            tx,
            rx,
            dead.clone(),
            error.clone(),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dead.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::yield_now();
        }
        drop(pump);
        assert!(dead.load(Ordering::Acquire));
        assert_eq!(error.lock().unwrap().as_deref(), Some("adapter removed"));
    }

    #[test]
    fn unrelated_receive_flood_cannot_prevent_shutdown() {
        let (tx, rx, _main) = handles();
        let dead = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let pump = Pump::start_with(
            || {
                Ok(FakeIo {
                    send_result: Ok(usize::MAX),
                    sent: Arc::default(),
                    receive_error: false,
                    flood: true,
                })
            },
            tx,
            rx,
            dead.clone(),
            error.clone(),
        )
        .unwrap();
        drop(pump);
        assert!(dead.load(Ordering::Acquire));
        assert!(error.lock().unwrap().is_none());
    }

    fn exercise_send(send_result: Result<usize, String>) -> Option<String> {
        let (tx, rx, main) = handles();
        let dead = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let pump = Pump::start_with(
            move || {
                Ok(FakeIo {
                    send_result,
                    sent: counter,
                    receive_error: false,
                    flood: false,
                })
            },
            tx,
            rx,
            dead.clone(),
            error.clone(),
        )
        .unwrap();
        // Poll the real protocol core until its first initialization PDU
        // is queued; the fake NIC deliberately never supplies a reply.
        let mut init = std::pin::pin!(main.init_single_group::<4, 64>(ethercat_now));
        assert!(smol::block_on(smol::future::poll_once(&mut init)).is_none());
        let deadline = Instant::now() + Duration::from_secs(2);
        while sent.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            sent.load(Ordering::Relaxed) > 0,
            "real EtherCrab PDU was not sent"
        );
        drop(pump);
        assert!(
            dead.load(Ordering::Acquire),
            "pump must stop with an unanswered PDU"
        );
        let message = error.lock().unwrap().clone();
        message
    }

    #[test]
    fn send_error_from_real_pdu_is_preserved() {
        assert!(exercise_send(Err("adapter send rejected".into()))
            .unwrap()
            .contains("adapter send rejected"));
    }

    #[test]
    fn partial_send_is_not_reported_as_success() {
        assert!(exercise_send(Ok(1)).unwrap().contains("PartialSend"));
    }

    #[test]
    fn unanswered_real_pdu_does_not_prevent_shutdown() {
        assert!(exercise_send(Ok(usize::MAX)).is_none());
    }

    #[test]
    fn returned_ethercat_frame_completes_real_register_read() {
        struct RegisterReply {
            frame: Vec<u8>,
            pending: bool,
        }
        impl PacketIo for RegisterReply {
            fn send(&mut self, bytes: &[u8]) -> Result<usize, String> {
                // One real FPRD with a two-byte register payload. Preserve
                // EtherCrab's generated datagram index and address; emulate
                // the slave's returned source MAC, value and working count.
                assert_eq!(bytes.len(), 30);
                assert_eq!(bytes[16], 4); // FPRD
                self.frame = bytes.to_vec();
                self.frame[6] |= 0x02;
                self.frame[26..28].copy_from_slice(&0x1234u16.to_le_bytes());
                self.frame[28..30].copy_from_slice(&1u16.to_le_bytes());
                self.pending = true;
                Ok(bytes.len())
            }
            fn receive(&mut self) -> Result<Option<&[u8]>, String> {
                if std::mem::take(&mut self.pending) {
                    Ok(Some(&self.frame))
                } else {
                    Ok(None)
                }
            }
        }
        let (tx, rx, main) = handles();
        let dead = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let pump = Pump::start_with(
            || {
                Ok(RegisterReply {
                    frame: Vec::new(),
                    pending: false,
                })
            },
            tx,
            rx,
            dead.clone(),
            error.clone(),
        )
        .unwrap();
        let value = smol::block_on(
            ethercrab::Command::fprd(0x1000, 0x0130)
                .with_wkc(1)
                .receive::<u16>(&main),
        )
        .unwrap();
        assert_eq!(value, 0x1234);
        drop(pump);
        assert!(dead.load(Ordering::Acquire));
        assert!(error.lock().unwrap().is_none());
    }

    #[test]
    fn packet_storage_owner_is_released_after_startup_failure() {
        let (tx, rx, _main) = handles();
        let storage_owner = Arc::new(());
        let weak = Arc::downgrade(&storage_owner);
        let result = Pump::start_owned::<FakeIo, _>(
            || Err("opening failed".into()),
            PacketHandles {
                tx,
                rx,
                _storage: storage_owner,
            },
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        );
        assert!(result.is_err());
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn detached_packet_call_keeps_storage_until_it_actually_exits() {
        struct BlockedDriver {
            entered: mpsc::SyncSender<()>,
            release: mpsc::Receiver<()>,
            closed: Arc<AtomicBool>,
        }
        impl PacketIo for BlockedDriver {
            fn send(&mut self, bytes: &[u8]) -> Result<usize, String> {
                Ok(bytes.len())
            }
            fn receive(&mut self) -> Result<Option<&[u8]>, String> {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
                Ok(None)
            }
        }
        impl Drop for BlockedDriver {
            fn drop(&mut self) {
                self.closed.store(true, Ordering::Release);
            }
        }
        let (tx, rx, _main) = handles();
        let storage_owner = Arc::new(());
        let weak = Arc::downgrade(&storage_owner);
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let closed = Arc::new(AtomicBool::new(false));
        let worker_closed = closed.clone();
        let dead = Arc::new(AtomicBool::new(false));
        let pump = Pump::start_owned(
            move || {
                Ok(BlockedDriver {
                    entered: entered_tx,
                    release: release_rx,
                    closed: worker_closed,
                })
            },
            PacketHandles {
                tx,
                rx,
                _storage: storage_owner,
            },
            dead.clone(),
            Arc::new(Mutex::new(None)),
        )
        .unwrap();
        let confirmed = pump.stop_confirmed.clone();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        drop(pump); // Deliberately exceeds the production join deadline.
        assert!(!confirmed.load(Ordering::Acquire));
        assert!(!dead.load(Ordering::Acquire));
        assert!(!closed.load(Ordering::Acquire));
        assert!(
            weak.upgrade().is_some(),
            "detached PDU handles must retain storage"
        );
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !dead.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert!(dead.load(Ordering::Acquire));
        assert!(closed.load(Ordering::Acquire));
        assert!(
            weak.upgrade().is_none(),
            "finished packet thread must release storage"
        );
    }

    #[test]
    fn full_capacity_init_and_typestate_futures_run_on_worker_stack_budget() {
        // Unlike a missing-driver test, this polls the real 128-slave/
        // 4096-byte futures, returns the large group by value and exercises
        // PREOP/PDI/OP/SAFEOP/DC conversion and destruction. An empty-bus
        // reply keeps the test offline while preserving full type sizes.
        struct EmptyBus {
            reply: Vec<u8>,
            pending: bool,
        }
        impl PacketIo for EmptyBus {
            fn send(&mut self, bytes: &[u8]) -> Result<usize, String> {
                self.reply = bytes.to_vec();
                self.reply[6] |= 0x02;
                self.pending = true;
                Ok(bytes.len())
            }
            fn receive(&mut self) -> Result<Option<&[u8]>, String> {
                Ok(std::mem::take(&mut self.pending).then_some(self.reply.as_slice()))
            }
        }
        thread::Builder::new()
            .name("ec-full-capacity-stack-test".into())
            .stack_size(crate::real::WINDOWS_WORKER_STACK_SIZE)
            .spawn(|| {
                let (tx, rx, main) = handles();
                let error = Arc::new(Mutex::new(None));
                let pump = Pump::start_with(
                    || {
                        Ok(EmptyBus {
                            reply: Vec::new(),
                            pending: false,
                        })
                    },
                    tx,
                    rx,
                    Arc::new(AtomicBool::new(false)),
                    error.clone(),
                )
                .unwrap();
                smol::block_on(Box::pin(async {
                    let group = Box::pin(main.init_single_group::<128, 4096>(ethercat_now))
                        .await
                        .unwrap();
                    assert!(group.is_empty());
                    let group = Box::pin(group.into_pre_op_pdi(&main)).await.unwrap();
                    let group = Box::pin(group.request_into_op(&main)).await.unwrap();
                    let group = Box::pin(group.into_safe_op(&main)).await.unwrap();
                    let group = Box::pin(group.into_pre_op(&main)).await.unwrap();
                    let group = Box::pin(group.into_op(&main)).await.unwrap();
                    let group = Box::pin(group.into_safe_op(&main)).await.unwrap();
                    let group = Box::pin(group.into_pre_op(&main)).await.unwrap();
                    let group = Box::pin(group.into_pre_op_pdi(&main)).await.unwrap();
                    let result = Box::pin(group.configure_dc_sync(
                        &main,
                        ethercrab::subdevice_group::DcConfiguration::default(),
                    ))
                    .await;
                    assert!(matches!(
                        result,
                        Err(ethercrab::error::Error::DistributedClock(
                            ethercrab::error::DistributedClockError::NoReference
                        ))
                    ));
                }));
                drop(pump);
                assert!(error.lock().unwrap().is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn full_capacity_init_cancels_after_first_real_pdu_and_releases_pump() {
        thread::Builder::new()
            .name("ec-full-capacity-cancel-test".into())
            .stack_size(crate::real::WINDOWS_WORKER_STACK_SIZE)
            .spawn(|| {
                let storage = Box::leak(Box::new(PduStorage::<8, 256>::new()));
                let (tx, rx, pdus) = storage.try_split().unwrap();
                let main = MainDevice::new(
                    pdus,
                    Timeouts {
                        // Give the explicit first-TX observation its own
                        // two-second deadline before the protocol timeout.
                        // This test must complete by cancellation, never
                        // by a failed EtherCAT response deadline.
                        pdu: Duration::from_secs(5),
                        ..Timeouts::default()
                    },
                    MainDeviceConfig::default(),
                );
                let sent = Arc::new(AtomicUsize::new(0));
                let worker_sent = sent.clone();
                let dead = Arc::new(AtomicBool::new(false));
                let error = Arc::new(Mutex::new(None));
                let storage_owner = Arc::new(());
                let weak_owner = Arc::downgrade(&storage_owner);
                let pump = Pump::start_owned(
                    move || {
                        Ok(FakeIo {
                            send_result: Ok(usize::MAX),
                            sent: worker_sent,
                            receive_error: false,
                            flood: false,
                        })
                    },
                    PacketHandles {
                        tx,
                        rx,
                        _storage: storage_owner,
                    },
                    dead.clone(),
                    error.clone(),
                )
                .unwrap();
                let confirmed = pump.stop_confirmed.clone();
                let shutdown = AtomicBool::new(false);
                {
                    let mut init = Box::pin(crate::real::cancel_init_for_test(
                        Box::pin(main.init_single_group::<128, 4096>(ethercat_now)),
                        &shutdown,
                    ));
                    assert!(smol::block_on(smol::future::poll_once(&mut init)).is_none());
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while sent.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(1));
                    }
                    assert!(
                        sent.load(Ordering::Acquire) > 0,
                        "initialization must send a real PDU before cancellation"
                    );
                    assert!(!dead.load(Ordering::Acquire));
                    shutdown.store(true, Ordering::Release);
                    assert!(
                        smol::block_on(init).is_none(),
                        "must return cancellation, not a protocol timeout"
                    );
                }
                drop(pump);
                assert!(dead.load(Ordering::Acquire));
                assert!(confirmed.load(Ordering::Acquire));
                assert!(weak_owner.upgrade().is_none());
                assert!(error.lock().unwrap().is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
