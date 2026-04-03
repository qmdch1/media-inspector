# Media Inspector

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=flat-square)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-CE422B?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%2B-0078D4?style=flat-square&logo=windows)](https://github.com/qmdch1/media-inspector/releases/latest)

A Windows media file management tool with file search, duplicate detection, and video quality analysis — all in one window.

## Download

[![Download](https://img.shields.io/github/v/release/qmdch1/media-inspector?label=Download&style=for-the-badge&logo=github&color=2ea44f)](https://github.com/qmdch1/media-inspector/releases/latest/download/MediaInspector.exe)

> Windows 10 x64 — no installation needed, just run the exe.

## Features

### File Search
- Recursively search filenames by keyword across multiple folders
- Press **Enter** or click **Search** to run
- Select a result and click **Open Location** to reveal it in Explorer

### Duplicate Finder
- Recursively scans folders and groups identical files by partial MD5 hash
- Select duplicates and delete them in bulk
- Shows total wasted space

### Video Quality Check
- ffprobe-based analysis covering 15 issue types (VFR, frame drops, codec compatibility, A/V sync, etc.)
- Three-tier classification: **Problem** / **Warning** / **OK**
- Configurable sample frame count and minimum file size
- Click a result to see details: resolution, codec, FPS, bitrate, issue list

### Common
- All three tabs share the same folder list — add folders once, use everywhere
- **Cancel** button stops any running scan immediately
- Dark theme UI (Segoe UI)

## Requirements

- Windows 10 or later (x86-64)
- **Video Quality Check**: `ffprobe.exe` from [FFmpeg](https://ffmpeg.org/download.html) must be in PATH or the same folder as the exe

## Build

```powershell
# Install Rust: https://rustup.rs
.\build.ps1
# Output: dist\MediaInspector.exe
```

Or directly:

```powershell
cargo build --release
# Output: target\release\MediaInspector.exe
```

## Issue Codes

| Code | Description |
|------|-------------|
| VFR | Variable frame rate (possible stuttering) |
| DROP | Frame drops detected |
| CORRUPT | Corrupted or missing frames |
| COMPAT | Codec/profile compatibility risk |
| AVSYNC | Audio/video sync error |
| BSPK | Bitrate spike |
| GOP | Keyframe interval too long (>10s) |
| LOWBR | Low bitrate relative to resolution |
| CTRMM | Container/codec mismatch |
| NOAUD | No audio stream |
| GOPI | Irregular keyframe intervals |
| DUR | Abnormal duration |
| HIBR | Abnormally high bitrate |
| RES | Non-standard resolution |
| ROT | Rotation metadata (portrait video) |

## License

[MIT](LICENSE)
