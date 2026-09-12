#!/bin/bash
if [ "${1:-}" = --version ]; then
  echo 'fake-cli 1.0'
  exit 0
fi
if [ "$(basename "$0")" = codex ]; then
  case " $* " in *' --json '*) ;; *) exit 90 ;; esac
  case " $* " in *' -s workspace-write '*) ;; *) exit 91 ;; esac
  case " $* " in *' sandbox_workspace_write.network_access=true '*) ;; *) exit 92 ;; esac
fi
if read -r _; then exit 93; fi
printf '%s\n' "$FAKE_EVENTS"
exit "$FAKE_STATUS"
