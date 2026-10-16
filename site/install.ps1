# Installs the newest TalkCraft release on Windows:
#
#   irm https://talkcraft.gerber.media/install.ps1 | iex
#
# Downloads the MSI from GitHub, checks it against SHA256SUMS.txt and runs it. Files fetched with
# PowerShell carry no "downloaded from the internet" mark, so SmartScreen doesn't block them;
# Windows still asks for administrator rights ("Publisher: Unknown").
# Runs inside a script block: under `iex`, `exit` would close the user's PowerShell window.
& {
  $ErrorActionPreference = 'Stop'
  $ProgressPreference = 'SilentlyContinue'  # Windows PowerShell 5 downloads very slowly with the progress bar
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  $repo = 'Gerber-Media/TalkCraft'
  $de = (Get-Culture).TwoLetterISOLanguageName -eq 'de'
  function Say($german, $english) { if ($de) { Write-Host $german } else { Write-Host $english } }
  function Fail($german, $english) { throw $(if ($de) { "TalkCraft: $german" } else { "TalkCraft: $english" }) }

  if (-not [Environment]::Is64BitOperatingSystem) { Fail 'TalkCraft braucht 64-Bit-Windows.' 'TalkCraft needs 64-bit Windows.' }

  $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'talkcraft-install' }
  $version = $release.tag_name.TrimStart('v')
  $msi = $release.assets | Where-Object { $_.name -match '^talkcraft-[\d.]+-windows-x64\.msi$' } | Select-Object -First 1
  $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS.txt' } | Select-Object -First 1
  if (-not $msi -or -not $sums) { Fail "Version $version hat keinen Windows-Installer." "release $version has no Windows installer." }
  Say "TalkCraft $version wird installiert." "Installing TalkCraft $version."

  $dir = Join-Path ([IO.Path]::GetTempPath()) ("talkcraft-" + [Guid]::NewGuid())
  New-Item -ItemType Directory -Path $dir | Out-Null
  try {
    $file = Join-Path $dir $msi.name
    Say "Lade $($msi.name) …" "Downloading $($msi.name) …"
    Invoke-WebRequest $msi.browser_download_url -OutFile $file -UseBasicParsing
    $line = (Invoke-RestMethod $sums.browser_download_url) -split "`n" | Where-Object { $_ -match ('\s\*?' + [Regex]::Escape($msi.name) + '\s*$') } | Select-Object -First 1
    if (-not $line) { Fail "$($msi.name) fehlt in SHA256SUMS.txt." "$($msi.name) is missing from SHA256SUMS.txt." }
    $want = ($line -split '\s+')[0].ToLower()
    $have = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($have -ne $want) { Fail 'Prüfsumme stimmt nicht, Abbruch.' 'checksum mismatch, aborting.' }
    Say 'Prüfsumme stimmt. Der Installer startet; bitte die Nachfrage von Windows mit „Ja“ bestätigen.' 'Checksum OK. The installer starts; please confirm the Windows prompt with "Yes".'
    $p = Start-Process msiexec.exe -ArgumentList '/i', "`"$file`"" -Wait -PassThru
    if ($p.ExitCode -eq 0 -or $p.ExitCode -eq 3010) {
      Say "TalkCraft $version ist installiert. Du findest es im Startmenü." "TalkCraft $version is installed. You'll find it in the Start menu."
    } elseif ($p.ExitCode -eq 1602) {
      Say 'Installation abgebrochen.' 'Installation cancelled.'
    } else {
      Fail "Der Installer ist mit Code $($p.ExitCode) beendet." "the installer exited with code $($p.ExitCode)."
    }
  } finally {
    Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
  }
}
