$ErrorActionPreference = 'Stop'
$renderRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../src-tauri/resources/renderer'))
if (Test-Path -LiteralPath $renderRoot) { throw "Renderer directory already exists: $renderRoot. Refusing to overwrite it." }
$stage = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) ('archgen-renderer-' + [guid]::NewGuid()))
# Pinned official release assets. Verify before unpacking or running anything.
$artifacts = @(
    @{ Name = 'plantuml.jar'; Url = 'https://github.com/plantuml/plantuml/releases/download/v1.2026.8/plantuml.jar'; Sha = '5e1ecfa8ecd32c90b03bbf3b1eb6f020943f98ab0fcf4032be31a0002ee2c462' },
    @{ Name = 'jre.zip'; Url = 'https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.12.1%2B1/OpenJDK21U-jre_x64_windows_hotspot_21.0.12.1_1.zip'; Sha = 'd35f31e712f0fcf6ac5a093edc90204fbff22f720ba3950bd09d331d5e621636' }
)
foreach ($artifact in $artifacts) {
    $target = Join-Path $stage.FullName $artifact.Name
    Invoke-WebRequest -Uri $artifact.Url -OutFile $target
    if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -ne $artifact.Sha) {
        throw "Checksum mismatch: $($artifact.Name). Staging files retained at $($stage.FullName)."
    }
}
Expand-Archive -LiteralPath (Join-Path $stage.FullName 'jre.zip') -DestinationPath (Join-Path $stage.FullName 'unpacked')
$jre = @(Get-ChildItem -LiteralPath (Join-Path $stage.FullName 'unpacked') -Directory)
if ($jre.Count -ne 1 -or !(Test-Path -LiteralPath (Join-Path $jre[0].FullName 'bin/java.exe'))) { throw 'Unexpected JRE archive layout.' }
New-Item -ItemType Directory -Path $renderRoot | Out-Null
Copy-Item -LiteralPath (Join-Path $stage.FullName 'plantuml.jar') -Destination $renderRoot
Copy-Item -LiteralPath $jre[0].FullName -Destination (Join-Path $renderRoot 'jre') -Recurse
Write-Output "Private renderer installed at $renderRoot. JRE legal notices retained. Download staging retained at $($stage.FullName)."
