// Bit-packing for the runtime write/force surface. Both the IDE server
// (`POST /api/runtime/variables/{name}`) and the edge runtime
// (`POST /write`) take an i32; the VM reads slots untyped, so a REAL
// write must arrive as its f32 bit pattern. Shared by the IDE api layer
// and the standalone HMI panel so the packing can't drift.

export function encodeForWrite(value: number, typeName: string): number {
  const t = typeName.toUpperCase()
  if (t === "REAL") {
    const buf = new ArrayBuffer(4)
    new Float32Array(buf)[0] = value
    return new Int32Array(buf)[0]
  }
  // BOOL, integer family, BYTE/WORD/DWORD all pass through as integers.
  return Math.trunc(value)
}
