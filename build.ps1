param(
    [string]$Clang = "clang",
    [string]$Target = "",
    [switch]$Clean
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$exeName = if ($IsWindows) { "snc.exe" } else { "snc" }
$exePath = Join-Path $root $exeName

if ($Clean) {
    Push-Location (Join-Path $root "compiler")
    cargo clean
    Pop-Location
    if (Test-Path $exePath) { Remove-Item -Force $exePath }
    return
}

Push-Location (Join-Path $root "compiler")
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
Pop-Location

$built = Join-Path $root "compiler/target/release/$exeName"
Copy-Item -Force $built $exePath
Write-Host "Built $exePath"
Write-Host "Compile a program: ./$exeName examples/hello_world.sn -o hello"
if ($Target) {
    Write-Host "Cross-compile a program with: ./$exeName file.sn --target $Target -o out --clang $Clang"
}
