# Release process

## Before SignPath approval

Release artifacts are unsigned and must be labeled **unsigned** in the
artifact name and release notes. They must never claim SignPath verification.

1. Create a version commit and `v*` tag from the default branch.
2. Let `.github/workflows/release-build.yml` build separate x64 and arm64 MSI
   installers on GitHub-hosted Windows runners.
3. Record the source commit and both SHA-256 digests.
4. Publish both installers and the source archive in one GitHub Release.

The release notes must include:

> This release is unsigned and has not been verified by SignPath Foundation.

## After SignPath approval

Add the SignPath signing-request step using only the organization, project,
artifact configuration, and signing policy supplied by SignPath. Store the API
token only as a GitHub Actions secret.

Each signed release requires manual approval by the signing approver and must
include:

> Free code signing provided by [SignPath.io](https://about.signpath.io),
> certificate by [SignPath Foundation](https://signpath.org).

Release notes must also identify:

- version and source commit;
- SHA-256 digests of the final x64 and arm64 signed installers;
- applicable changes;
- link to the [Code signing policy](CODE_SIGNING_POLICY.md);
- link to the [Privacy policy](PRIVACY.md);
- link to the source archive for the same tag.

Signed files must not be modified or replaced after publication.
