# Release process

Releases are currently published without code signing. Nothing in the release
may claim SignPath verification or any other signature.

1. Create a version commit and `v*` tag from the default branch.
2. Let `.github/workflows/release-build.yml` build separate x64 and arm64 NSIS
   installers on GitHub-hosted Windows runners.
3. Record the source commit and both SHA-256 digests.
4. Publish both installers and the source archive in one GitHub Release.

Release notes must identify:

- version and source commit;
- SHA-256 digests of the x64 and arm64 installers;
- applicable changes;
- link to the [Privacy policy](PRIVACY.md);
- link to the source archive for the same tag.

Published files must not be modified or replaced afterwards.

Releases ship the NSIS installer, whose uninstaller offers a "delete application
data" checkbox wired up in `runtime/windows-nsis-hooks.nsh`. The MSI produced by
`run-store-release.ps1 -Bundle msi` exists only for Microsoft Store submission —
its uninstall path leaves user data behind and offers no way to remove it.

## If code signing is adopted later

[CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md) describes the SignPath
Foundation route. Adopting it means adding the signing-request step to the
workflow, keeping the API token only as a GitHub Actions secret, requiring
manual approval by the signing approver, and adding to each release:

> Free code signing provided by [SignPath.io](https://about.signpath.io),
> certificate by [SignPath Foundation](https://signpath.org).