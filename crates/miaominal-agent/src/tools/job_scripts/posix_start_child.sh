#!/bin/sh
@@HELPERS@@
child_meta=@@CHILD@@
saved_umask=$(umask)
child_pid=$$
child_uid=$(id -u 2>/dev/null) || exit 125
child_pgid=$(process_pgid "$child_pid") || exit 125
child_identity=$(process_identity "$child_pid") || exit 125
umask 077
child_tmp="$child_meta.tmp.$child_pid"
{
    printf 'pid=%s\n' "$child_pid"
    printf 'uid=%s\n' "$child_uid"
    printf 'pgid=%s\n' "$child_pgid"
    printf 'identity=%s\n' "$child_identity"
} >"$child_tmp" && mv -f "$child_tmp" "$child_meta"
umask "$saved_umask"
cd "$HOME" && cd @@CWD@@ || exit 126
exec sh -lc @@COMMAND@@
