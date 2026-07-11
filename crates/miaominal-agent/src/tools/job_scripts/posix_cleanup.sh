root=@@ROOT@@
if [ -L "$root" ] || [ ! -d "$root" ]; then exit 1; fi
rm -f @@STATUS@@ @@STDOUT@@ @@STDERR@@ @@PID@@ @@READY@@ @@RUNNER@@ @@COMMAND@@ @@CHILD@@ @@STOP@@ @@ERROR@@
rm -f "$root"/status.tmp.* "$root"/pid.tmp.* "$root"/ready.tmp.* "$root"/error.tmp.* "$root"/child.tmp.* 2>/dev/null || true
rmdir "$root"
