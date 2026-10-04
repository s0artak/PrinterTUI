# Windows: signing and package managers

## Scoop (done)

Every release carries `printertui.json`, a Scoop manifest made by `packaging/scoop.sh` in the
release job. It installs and updates straight from the release:

```powershell
scoop install https://github.com/s0artak/PrinterTUI/releases/latest/download/printertui.json
```

The manifest has `checkver` and `autoupdate`, so it can also go into a bucket as it is (a repo
`scoop-printertui` with it under `bucket/`, then `scoop bucket add printertui <url>`).

## winget (needs the owner, once)

winget installs only what is in [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs).
The first version is submitted by hand; later ones can be automated.

1. After a release, on Windows: `winget install wingetcreate`, then
   `wingetcreate new https://github.com/s0artak/PrinterTUI/releases/download/vX.Y.Z/printertui-windows-x86_64.exe`.
   It asks for the package id (`s0artak.PrinterTUI`), name, publisher and license, and detects a
   portable exe (the installer type winget uses for a single exe; it puts it on the PATH).
2. Add the ARM installer to the manifest it writes (same release, `printertui-windows-aarch64.exe`).
3. `wingetcreate submit` opens the pull request on winget-pkgs; a Microsoft bot validates it.
4. For later releases: a fine-grained token with access to a fork of winget-pkgs as the secret
   `WINGET_TOKEN`, and a release step with `vedantmgoyal9/winget-releaser` (or
   `wingetcreate update s0artak.PrinterTUI --urls ... --submit --token ...`).

## Code signing (needs the owner)

Unsigned, `printertui.exe` makes SmartScreen warn ("Windows protected your PC") on first run until
enough people have run it. Signing removes the warning for good (with an EV certificate) or after
some reputation (with a standard one).

- Free for open source: [SignPath Foundation](https://signpath.org). Apply with the repository;
  they sign builds made by GitHub Actions. The release job then sends the two exes to
  `signpath/github-action-submit-signing-request` and publishes the signed ones (their `.sha256`
  must be made after signing, as install.ps1 checks it).
- Paid: an OV/EV certificate (or Azure Trusted Signing, about $10 a month) and `signtool sign /fd sha256 /tr <timestamp url> /td sha256 printertui-windows-*.exe`
  in the Windows build jobs, before the `.sha256` step.
