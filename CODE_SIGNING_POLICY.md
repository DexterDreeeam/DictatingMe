# Code signing policy

## Service

Free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org).

SignPath Foundation approval has not yet been granted. Until approval and
workflow integration are complete, release artifacts must be described as
unsigned and must not claim SignPath verification.

## Roles

- Committer and reviewer:
  [DexterDreeeam](https://github.com/DexterDreeeam)
- Signing approver:
  [DexterDreeeam](https://github.com/DexterDreeeam)

Repository and SignPath accounts used by these roles must have multi-factor
authentication enabled.

## Source and build policy

1. Release binaries are built only from this public repository.
2. `Cargo.lock` and `package-lock.json` are committed and used during builds.
3. x64 and arm64 release builds run on GitHub-hosted Windows runners after
   SignPath integration is approved.
4. The unsigned artifact is uploaded by the same workflow before a signing
   request is submitted.
5. Every signing request requires manual approval by the signing approver.
6. Build scripts, release workflows, SignPath policies, dependency manifests,
   and lockfiles receive the same review as application source code.
7. Signed files are not modified after signing.
8. Every release publishes its version, source commit, architectures, and
   SHA-256 digests.

The SignPath organization ID, project slug, signing policy slug, and API token
will be configured only after SignPath Foundation accepts the project.

## Privacy statement

DictatingMe does not transfer microphone audio, transcriptions, history, or
speaker profiles to the project maintainer or any analytics service. Network
access occurs when a user explicitly downloads an optional model from a source
listed in `assets/manifest-cn.json`. See [PRIVACY.md](PRIVACY.md).
