# Third-party notices

DictatingMe is licensed under Apache-2.0. Third-party software and model
artifacts retain their own licenses.

## Runtime and build dependencies

Rust and Node dependencies are declared in `runtime/Cargo.toml`,
`xtask/Cargo.toml`, and `package.json`; exact versions are recorded in
`Cargo.lock` and `package-lock.json`. Their package metadata declares
OSI-approved or equivalent permissive licenses, including Apache-2.0, MIT,
MPL-2.0, BSD, ISC, Unicode-3.0, Zlib, BSL-1.0, and compatible combinations.

Major components include:

- [Tauri](https://github.com/tauri-apps/tauri), Apache-2.0 or MIT.
- [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), Apache-2.0.
- [Tokio](https://github.com/tokio-rs/tokio), MIT.
- [Rodio](https://github.com/RustAudio/rodio), Apache-2.0 or MIT.
- [CPAL](https://github.com/RustAudio/cpal), Apache-2.0.
- [rusqlite](https://github.com/rusqlite/rusqlite), MIT.
- [Vite](https://github.com/vitejs/vite), MIT.
- [TypeScript](https://github.com/microsoft/TypeScript), Apache-2.0.

## Speech and speaker models

- `sherpa-onnx-kws-zipformer-wenetspeech-3.3M-2024-01-01`: Apache-2.0;
  derived from the Apache-2.0 WenetSpeech project and distributed by
  sherpa-onnx.
- `sherpa-onnx-streaming-zipformer-bilingual-zh-en-2023-02-20`:
  Apache-2.0; distributed by sherpa-onnx.
- `sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25`: based on
  [Qwen3-ASR](https://github.com/QwenLM/Qwen3-ASR), Apache-2.0, and
  distributed in sherpa-onnx format.
- `3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx`: from
  [3D-Speaker](https://github.com/modelscope/3D-Speaker), Apache-2.0.

Download URLs and immutable expected hashes are maintained in
`assets/manifest-cn.json` and `assets/sha.json`.

Windows x64 and arm64 builds link the official sherpa-onnx v1.13.4 static
library archives published by k2-fsa. Their SHA-256 values are pinned in
`assets/run-store-release.ps1`.

Development-only noise and synthetic test corpora under ignored `assets/`
subdirectories are not part of the source repository or release installer.
