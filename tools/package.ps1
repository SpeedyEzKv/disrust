[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$project = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location $project
try {
    $meta = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc | ConvertFrom-Json
    if ($LASTEXITCODE) { throw 'Could not read Cargo dependencies.' }

    $binary = 'target/release/noir-chat.exe'
    if (!(Test-Path -LiteralPath $binary -PathType Leaf)) {
        throw 'Run cargo build --release --locked before packaging.'
    }

    New-Item -Path dist -ItemType Directory -Force | Out-Null
    $package = Join-Path $project 'dist/Noir-Chat-windows-x64.zip'
    $temporary = "$package.tmp"
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    try {
        $archive = [IO.Compression.ZipFile]::Open($temporary, [IO.Compression.ZipArchiveMode]::Create)
        try {
            $files = @{
                'dist/Noir-Chat.exe' = $binary
                'README.md' = 'README.md'
                'assets/avatar.png' = 'assets/avatar.png'
            }
            foreach ($entry in $files.GetEnumerator()) {
                $source = Join-Path $project $entry.Value
                [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                    $archive, $source, $entry.Key, [IO.Compression.CompressionLevel]::Optimal
                ) | Out-Null
            }

            $index = foreach ($dependency in ($meta.packages | Where-Object source | Sort-Object name,version)) {
                $folder = "third-party/$($dependency.name)-$($dependency.version)"
                $dependencyPath = Split-Path -Parent $dependency.manifest_path
                $notices = Get-ChildItem -LiteralPath $dependencyPath -File |
                    Where-Object Name -Match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT)([_.-]|$)'
                foreach ($notice in $notices) {
                    [IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
                        $archive, $notice.FullName, "$folder/$($notice.Name)", [IO.Compression.CompressionLevel]::Optimal
                    ) | Out-Null
                }
                "$($dependency.name) $($dependency.version)  $($dependency.license)"
            }
            $writer = [IO.StreamWriter]::new($archive.CreateEntry('third-party/INDEX.txt').Open())
            try { $writer.WriteLine(($index -join [Environment]::NewLine)) }
            finally { $writer.Dispose() }
        } finally {
            $archive.Dispose()
        }

        $verify = [IO.Compression.ZipFile]::OpenRead($temporary)
        try {
            $entry = $verify.GetEntry('dist/Noir-Chat.exe').Open()
            $hash = [Security.Cryptography.SHA256]::Create()
            try {
                $packed = [BitConverter]::ToString($hash.ComputeHash($entry)).Replace('-', '')
            } finally {
                $hash.Dispose()
                $entry.Dispose()
            }
            if ($packed -ne (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash) {
                throw 'The ZIP executable differs from the release build.'
            }
        } finally {
            $verify.Dispose()
        }
        Move-Item -LiteralPath $temporary -Destination $package -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
    }
    Get-FileHash -LiteralPath $package -Algorithm SHA256
} finally {
    Pop-Location
}
