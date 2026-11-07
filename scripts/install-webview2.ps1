$ErrorActionPreference = 'Stop'
$clientKey = 'Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
foreach ($key in @("HKLM:\SOFTWARE\$clientKey", "HKLM:\SOFTWARE\WOW6432Node\$clientKey", "HKCU:\SOFTWARE\$clientKey")) {
    $version = (Get-ItemProperty -LiteralPath $key -ErrorAction SilentlyContinue).pv
    if ($version -and [version]$version -gt [version]'0.0.0.0') { exit 0 }
}
$bootstrapRoot = Join-Path ([IO.Path]::GetTempPath()) ('rf-webview2-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $bootstrapRoot | Out-Null
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $installer = Join-Path $bootstrapRoot 'MicrosoftEdgeWebview2Setup.exe'
    Invoke-WebRequest -UseBasicParsing -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $installer
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
        throw 'WebView2 bootstrapper signature verification failed'
    }
    $result = Start-Process -FilePath $installer -ArgumentList '/silent', '/install' -WindowStyle Hidden -Wait -PassThru
    if ($result.ExitCode -notin @(0, 3010)) { throw "WebView2 installation failed: $($result.ExitCode)" }
} finally {
    if (-not [IO.Path]::GetFullPath($bootstrapRoot).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Unexpected bootstrap cleanup directory'
    }
    Remove-Item -LiteralPath $bootstrapRoot -Recurse -Force
}
