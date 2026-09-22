# Post-build artifact verification for the WinOptimizer Free release.
#
# Inspects the distributable and rejects commercial implementation identity,
# source maps, PDB files, and developer profile paths.

param(
    [string]$Dist = (Join-Path $PSScriptRoot '..\dist')
)

$ErrorActionPreference = 'Stop'
$failures = New-Object System.Collections.Generic.List[string]

# Assembled at runtime so this checker does not contain the identifiers it forbids.
$identity = @(
    ('winoptimizer' + '-' + 'dedup'),
    ('winoptimizer' + '-' + 'pro'),
    ('winoptimizer' + '-' + 'commercial'),
    ('media' + '_' + 'fingerprint'),
    ('duplicate' + '_' + 'operation'),
    ('media' + '_' + 'duplicate' + '_' + 'cleanup'),
    ('find' + '_' + 'duplicate' + '_' + 'files')
)
$signatures = @(
    ('perceptual' + ' ' + 'hash'),
    ('audio' + '_' + 'fingerprint'),
    ('video' + '_' + 'fingerprint'),
    ('dedup' + ' ' + 'engine')
)
$allTokens = $identity + $signatures

if (-not (Test-Path -LiteralPath $Dist)) {
    throw "dist directory not found: $Dist"
}

$files = Get-ChildItem -LiteralPath $Dist -Recurse -File
if ($files.Count -eq 0) {
    throw "dist directory is empty: $Dist"
}

function Get-AsciiText([string]$path) {
    $bytes = [System.IO.File]::ReadAllBytes($path)
    return [System.Text.Encoding]::ASCII.GetString($bytes)
}

foreach ($file in $files) {
    if ($file.Name -like '*.map') {
        $failures.Add("source map present: $($file.Name)")
    }
    if ($file.Extension -eq '.pdb') {
        $failures.Add("PDB file present: $($file.Name)")
    }
    if ($file.Extension -in @('.exe', '.dll', '.js', '.mjs', '.cjs', '.json', '.html', '.css')) {
        $text = Get-AsciiText $file.FullName
        foreach ($token in $allTokens) {
            if ($text.Contains($token)) {
                $failures.Add("forbidden token '$token' in $($file.Name)")
            }
        }
        if ($file.Extension -in @('.exe', '.dll')) {
            if ($text -match '(?i)[A-Za-z]:\\Users\\[^"\x00]{1,120}' -and
                $text -notmatch '(?i)[A-Za-z]:\\Users\\Default') {
                $failures.Add("developer profile path embedded in $($file.Name)")
            }
        }
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) { Write-Error "artifact verification: $failure" }
    exit 1
}

Write-Output "artifact verification passed ($($files.Count) packaged files scanned)"
