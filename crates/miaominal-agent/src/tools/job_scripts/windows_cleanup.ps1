$marker=[Environment]::ExpandEnvironmentVariables(@@MARKER@@)
Remove-Item -LiteralPath @($marker,($marker+'.out'),($marker+'.err'),($marker+'.pid'),($marker+'.ctl.out'),($marker+'.ctl.err'),($marker+'.runner.ps1')) -Force -ErrorAction SilentlyContinue
$root=Split-Path -Parent $marker; $leaf=Split-Path -Leaf $marker
Get-ChildItem -LiteralPath $root -Filter ($leaf+'.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
Get-ChildItem -LiteralPath $root -Filter ($leaf+'.pid.tmp-*') -File -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
