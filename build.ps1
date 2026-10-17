# Build ASCII Painter for Windows x64 into dist\ascii-painter-windows-x64.zip.
# The zip is portable: ascii-painter.exe (GUI), ascii-painter-cli.exe and a bundled ffmpeg/ffprobe.
# Usage: .\build.ps1
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

# ffmpeg 9.0.2 essentials build (GPL, includes libx264), pinned by SHA-256.
$FfmpegUrl = 'https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/ffmpeg-9.0.2-essentials_build.zip'
$FfmpegSha256 = '60F467265B1E312373DBCD92200C2618A74850F98D3D078E94296BB3FA2047BA'
$FfmpegDir = 'ffmpeg-9.0.2-essentials_build/bin'

# Own target folder: some security software (e.g. Norton) keeps blocking build-script paths in
# target\release it once flagged, even after the folder is excluded.
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target\package' }
$name = 'ascii-painter-windows-x64'
$out = "dist/$name"
$cache = 'dist/cache'

Write-Host '==> Building (release)'
cargo build --release --workspace --target-dir $target
if ($LASTEXITCODE -ne 0) {
    throw @"
cargo build failed. If the error is "failed to link or copy ... build-script-build.exe ... Access denied",
security software (e.g. Norton) is blocking cargo's build scripts at those paths. Remove the blocked items
from its history/quarantine, or build in a fresh folder: `$env:CARGO_TARGET_DIR='C:\temp\ascii-painter-target'; .\build.ps1
"@
}

Write-Host '==> Fetching ffmpeg'
New-Item -ItemType Directory -Force $cache | Out-Null
$archive = "$cache/ffmpeg-windows.zip"
if (-not (Test-Path $archive) -or (Get-FileHash $archive -Algorithm SHA256).Hash -ne $FfmpegSha256) {
    Invoke-WebRequest -Uri $FfmpegUrl -OutFile "$archive.part"
    $actual = (Get-FileHash "$archive.part" -Algorithm SHA256).Hash
    if ($actual -ne $FfmpegSha256) {
        Remove-Item "$archive.part"
        throw "ffmpeg checksum mismatch: expected $FfmpegSha256, got $actual"
    }
    Move-Item -Force "$archive.part" $archive
}

Write-Host "==> Assembling $out"
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item "$target/release/ascii-painter.exe", "$target/release/ascii-painter-cli.exe", README.md $out
Copy-Item -Recurse LICENSES $out
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path $archive))
try {
    foreach ($tool in 'ffmpeg.exe', 'ffprobe.exe') {
        $entry = $zip.Entries | Where-Object { $_.FullName -eq "$FfmpegDir/$tool" }
        if (-not $entry) { throw "$tool not found in the ffmpeg archive" }
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path (Resolve-Path $out) $tool), $true)
    }
} finally {
    $zip.Dispose()
}

Compress-Archive -Path $out -DestinationPath "dist/$name.zip" -Force
Write-Host "==> Done: dist/$name.zip (run $out/ascii-painter.exe)"
