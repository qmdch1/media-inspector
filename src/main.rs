#![windows_subsystem = "windows"]

use std::collections::HashMap;
use std::io::Read as IoRead;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::{
    core::*,
    Win32::Foundation::*,
    Win32::Graphics::Gdi::*,
    Win32::System::Com::*,
    Win32::System::LibraryLoader::*,
    Win32::UI::Controls::*,
    Win32::UI::Input::KeyboardAndMouse::*,
    Win32::UI::Shell::*,
    Win32::UI::WindowsAndMessaging::*,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── 탭 모드 ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    FileFinder,
    DuplicateFinder,
    VideoChecker,
}

// ── 컨트롤 ID ─────────────────────────────────────────────────────────────────

// 공통
const ID_TAB_FILE:   u16 = 10;
const ID_TAB_DUP:    u16 = 11;
const ID_TAB_VIDEO:  u16 = 12;

// 공유 (모든 탭에서 동일한 역할)
const ID_FOLDER_LIST: u16 = 101;
const ID_BTN_ADD:     u16 = 102;
const ID_BTN_REMOVE:  u16 = 103;
const ID_RESULT_LIST: u16 = 107;
const ID_STATUS:      u16 = 108;
const ID_STATS:       u16 = 109;
const ID_BTN_CANCEL:  u16 = 106;

// 파일 검색 전용
const ID_EDIT_QUERY:  u16 = 104;
const ID_BTN_SEARCH:  u16 = 105;
const ID_BTN_OPEN:    u16 = 110;

// 중복 검색 전용
const ID_BTN_SCAN_DUP:  u16 = 120;
const ID_BTN_DELETE:    u16 = 121;

// 영상 체크 전용
const ID_BTN_SCAN_VID:  u16 = 130;
const ID_EDIT_FRAMES:   u16 = 131;
const ID_EDIT_SIZE:     u16 = 132;
const ID_DETAIL_TEXT:   u16 = 133;
const ID_BTN_OPEN_VID:  u16 = 134;

// ── 메시지 ────────────────────────────────────────────────────────────────────

const WM_SEARCH_DONE:    u32 = WM_APP + 1;
const WM_SCAN_DONE_DUP:  u32 = WM_APP + 2;
const WM_SCAN_PROGRESS:  u32 = WM_APP + 3;
const WM_SCAN_DONE_VID:  u32 = WM_APP + 4;

// ── 컬러 ─────────────────────────────────────────────────────────────────────

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

// ─────────────────────────────────────────────────────────────────────────────
// FILE FINDER 로직
// ─────────────────────────────────────────────────────────────────────────────

struct SearchResult {
    files: Vec<PathBuf>,
    total_searched: usize,
    elapsed_secs: f64,
    cancelled: bool,
}

fn file_matches(file_name: &str, keyword: &str) -> bool {
    file_name.to_lowercase().contains(&keyword.to_lowercase())
}

fn run_file_search(folders: Vec<PathBuf>, keyword: String, cancel: Arc<Mutex<bool>>) -> SearchResult {
    let start = Instant::now();
    let mut files = Vec::new();
    let mut total_searched = 0usize;

    for folder in &folders {
        collect_file_matches(folder, &keyword, &mut files, &mut total_searched, &cancel);
        if *cancel.lock().unwrap() {
            return SearchResult { files, total_searched, elapsed_secs: start.elapsed().as_secs_f64(), cancelled: true };
        }
    }
    files.sort();
    SearchResult { files, total_searched, elapsed_secs: start.elapsed().as_secs_f64(), cancelled: false }
}

fn collect_file_matches(
    dir: &Path, keyword: &str, results: &mut Vec<PathBuf>,
    count: &mut usize, cancel: &Arc<Mutex<bool>>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *cancel.lock().unwrap() { return; }
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if path.is_dir() {
            if !name.contains("recycle") {
                collect_file_matches(&path, keyword, results, count, cancel);
            }
        } else {
            *count += 1;
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if file_matches(&file_name, keyword) {
                results.push(path);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DUPLICATE FINDER 로직
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct FileEntry {
    path: PathBuf,
    size: u64,
}

#[derive(Clone)]
struct DupGroup {
    size: u64,
    files: Vec<FileEntry>,
}

struct DupScanResult {
    groups: Vec<DupGroup>,
    total_files: usize,
    elapsed_secs: f64,
    cancelled: bool,
}

fn partial_md5(path: &Path) -> Option<[u8; 16]> {
    let mut f = std::fs::File::open(path).ok()?;
    let meta = f.metadata().ok()?;
    let size = meta.len();
    let mut h = md5_state();

    const CHUNK: usize = 4 * 1024 * 1024;
    let mut buf = vec![0u8; CHUNK];

    let n = f.read(&mut buf).ok()?;
    md5_update(&mut h, &buf[..n]);

    if size > (CHUNK * 2) as u64 {
        use std::io::Seek;
        f.seek(std::io::SeekFrom::End(-(CHUNK as i64))).ok()?;
        let n = f.read(&mut buf).ok()?;
        md5_update(&mut h, &buf[..n]);
    }
    md5_update(&mut h, &size.to_be_bytes());
    Some(md5_final(h))
}

type Md5State = [u32; 4];

fn md5_state() -> Md5State { [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476] }

fn md5_update(state: &mut Md5State, data: &[u8]) {
    const S: [u32; 64] = [
        7,12,17,22,7,12,17,22,7,12,17,22,7,12,17,22,
        5, 9,14,20,5, 9,14,20,5, 9,14,20,5, 9,14,20,
        4,11,16,23,4,11,16,23,4,11,16,23,4,11,16,23,
        6,10,15,21,6,10,15,21,6,10,15,21,6,10,15,21,
    ];
    const K: [u32; 64] = [
        0xd76aa478,0xe8c7b756,0x242070db,0xc1bdceee,0xf57c0faf,0x4787c62a,0xa8304613,0xfd469501,
        0x698098d8,0x8b44f7af,0xffff5bb1,0x895cd7be,0x6b901122,0xfd987193,0xa679438e,0x49b40821,
        0xf61e2562,0xc040b340,0x265e5a51,0xe9b6c7aa,0xd62f105d,0x02441453,0xd8a1e681,0xe7d3fbc8,
        0x21e1cde6,0xc33707d6,0xf4d50d87,0x455a14ed,0xa9e3e905,0xfcefa3f8,0x676f02d9,0x8d2a4c8a,
        0xfffa3942,0x8771f681,0x6d9d6122,0xfde5380c,0xa4beea44,0x4bdecfa9,0xf6bb4b60,0xbebfbc70,
        0x289b7ec6,0xeaa127fa,0xd4ef3085,0x04881d05,0xd9d4d039,0xe6db99e5,0x1fa27cf8,0xc4ac5665,
        0xf4292244,0x432aff97,0xab9423a7,0xfc93a039,0x655b59c3,0x8f0ccc92,0xffeff47d,0x85845dd1,
        0x6fa87e4f,0xfe2ce6e0,0xa3014314,0x4e0811a1,0xf7537e82,0xbd3af235,0x2ad7d2bb,0xeb86d391,
    ];

    let mut padded = data.to_vec();
    let orig_len = data.len();
    padded.push(0x80);
    while padded.len() % 64 != 56 { padded.push(0); }
    let bit_len = (orig_len as u64) * 8;
    padded.extend_from_slice(&bit_len.to_le_bytes());

    let [mut a0, mut b0, mut c0, mut d0] = *state;
    for chunk in padded.chunks(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes(chunk[i*4..i*4+4].try_into().unwrap());
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0u32..64 {
            let (f, g) = match i {
                0..=15  => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5*i+1)%16),
                32..=47 => (b ^ c ^ d,           (3*i+5)%16),
                _       => (c ^ (b | !d),          (7*i)%16),
            };
            let temp = d; d = c; c = b;
            b = b.wrapping_add((a.wrapping_add(f).wrapping_add(K[i as usize]).wrapping_add(m[g as usize])).rotate_left(S[i as usize]));
            a = temp;
        }
        a0 = a0.wrapping_add(a); b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c); d0 = d0.wrapping_add(d);
    }
    *state = [a0, b0, c0, d0];
}

fn md5_final(state: Md5State) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, &v) in state.iter().enumerate() {
        out[i*4..i*4+4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

fn run_dup_scan(folders: Vec<PathBuf>, cancel: Arc<Mutex<bool>>) -> DupScanResult {
    let start = Instant::now();
    let mut size_map: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    let mut total_files = 0usize;

    for folder in &folders {
        collect_dup_files(folder, &mut size_map, &mut total_files, &cancel);
        if *cancel.lock().unwrap() {
            return DupScanResult { groups: vec![], total_files, elapsed_secs: start.elapsed().as_secs_f64(), cancelled: true };
        }
    }

    let mut groups = Vec::new();
    for (_size, files) in size_map.into_iter().filter(|(s, f)| *s > 0 && f.len() >= 2) {
        if *cancel.lock().unwrap() { break; }
        let mut hash_map: HashMap<[u8; 16], Vec<FileEntry>> = HashMap::new();
        for entry in files {
            if *cancel.lock().unwrap() { break; }
            if let Some(hash) = partial_md5(&entry.path) {
                hash_map.entry(hash).or_default().push(entry);
            }
        }
        for (_, dup_files) in hash_map.into_iter().filter(|(_, f)| f.len() >= 2) {
            let size = dup_files[0].size;
            groups.push(DupGroup { size, files: dup_files });
        }
    }
    groups.sort_by(|a, b| b.size.cmp(&a.size));

    DupScanResult { groups, total_files, elapsed_secs: start.elapsed().as_secs_f64(), cancelled: false }
}

fn collect_dup_files(
    dir: &Path, size_map: &mut HashMap<u64, Vec<FileEntry>>,
    count: &mut usize, cancel: &Arc<Mutex<bool>>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *cancel.lock().unwrap() { return; }
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if path.is_dir() {
            if !name.contains("recycle") {
                collect_dup_files(&path, size_map, count, cancel);
            }
        } else if let Ok(meta) = entry.metadata() {
            let size = meta.len();
            size_map.entry(size).or_default().push(FileEntry { path, size });
            *count += 1;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VIDEO CHECKER 로직
// ─────────────────────────────────────────────────────────────────────────────

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm",
    "m4v", "ts", "mts", "m2ts", "vob", "mpg", "mpeg",
    "3gp", "3g2", "rm", "rmvb", "divx", "xvid", "ogv",
];

#[derive(Clone, Debug, PartialEq)]
enum IssueKind {
    Vfr, FrameDrop, CorruptFrame, CodecCompat, AvSync, BitrateSpike,
    GopTooLong, LowBitrate, ContainerMismatch, NoAudio, GopIrregular,
    AbnormalDuration, HighBitrate, UnusualResolution, RotationTag,
}

impl IssueKind {
    fn code(&self) -> &'static str {
        match self {
            IssueKind::Vfr               => "VFR",
            IssueKind::FrameDrop         => "DROP",
            IssueKind::CorruptFrame      => "CORRUPT",
            IssueKind::CodecCompat       => "COMPAT",
            IssueKind::AvSync            => "AVSYNC",
            IssueKind::BitrateSpike      => "BSPK",
            IssueKind::GopTooLong        => "GOP",
            IssueKind::LowBitrate        => "LOWBR",
            IssueKind::ContainerMismatch => "CTRMM",
            IssueKind::NoAudio           => "NOAUD",
            IssueKind::GopIrregular      => "GOPI",
            IssueKind::AbnormalDuration  => "DUR",
            IssueKind::HighBitrate       => "HIBR",
            IssueKind::UnusualResolution => "RES",
            IssueKind::RotationTag       => "ROT",
        }
    }

    fn score(&self) -> u32 {
        match self {
            IssueKind::Vfr | IssueKind::FrameDrop | IssueKind::CorruptFrame => 25,
            IssueKind::CodecCompat | IssueKind::AvSync => 20,
            IssueKind::BitrateSpike | IssueKind::GopTooLong | IssueKind::LowBitrate | IssueKind::ContainerMismatch => 15,
            IssueKind::NoAudio | IssueKind::GopIrregular | IssueKind::AbnormalDuration => 10,
            IssueKind::HighBitrate | IssueKind::UnusualResolution | IssueKind::RotationTag => 5,
        }
    }

    fn description(&self) -> &'static str {
        match self {
            IssueKind::Vfr               => "Variable frame rate (possible stuttering)",
            IssueKind::FrameDrop         => "Frame drops detected",
            IssueKind::CorruptFrame      => "Corrupted/missing frames",
            IssueKind::CodecCompat       => "Codec/profile compatibility risk",
            IssueKind::AvSync            => "Audio/video sync error",
            IssueKind::BitrateSpike      => "Bitrate spike",
            IssueKind::GopTooLong        => "Keyframe interval too long (>10s)",
            IssueKind::LowBitrate        => "Low bitrate relative to resolution",
            IssueKind::ContainerMismatch => "Container/codec mismatch",
            IssueKind::NoAudio           => "No audio stream",
            IssueKind::GopIrregular      => "Irregular keyframe intervals",
            IssueKind::AbnormalDuration  => "Abnormal duration",
            IssueKind::HighBitrate       => "Abnormally high bitrate",
            IssueKind::UnusualResolution => "Non-standard resolution",
            IssueKind::RotationTag       => "Rotation metadata (portrait video)",
        }
    }
}

#[derive(Clone)]
struct VideoInfo {
    path: PathBuf,
    format_name: String, format_duration: f64, format_bit_rate: u64,
    codec_name: String, codec_profile: String,
    width: u32, height: u32, avg_frame_rate: f64,
    stream_duration: f64, video_bit_rate: u64, pix_fmt: String,
    rotation: i32, video_start_time: f64,
    has_audio: bool, audio_start_time: f64,
    frame_pts: Option<Vec<f64>>,
    frame_pkt_sizes: Option<Vec<u64>>,
    keyframe_indices: Option<Vec<usize>>,
}

#[derive(Clone, PartialEq)]
enum ResultCategory { Problem, Warning, Normal }

#[derive(Clone)]
struct AnalysisResult {
    video: VideoInfo,
    issues: Vec<IssueKind>,
    severity: u32,
    category: ResultCategory,
}

impl AnalysisResult {
    fn label(&self) -> String {
        let name = self.video.path.file_name().unwrap_or_default().to_string_lossy();
        if self.issues.is_empty() {
            format!("  [OK]  {}", name)
        } else {
            let codes: Vec<&str> = self.issues.iter().map(|i| i.code()).collect();
            format!("  [{}]  {}  (score: {})", codes.join("]["), name, self.severity)
        }
    }
}

fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 { return 0.0; }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    var.sqrt()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() { return 0.0; }
    values.iter().sum::<f64>() / values.len() as f64
}

fn parse_rational(s: &str) -> f64 {
    let parts: Vec<&str> = s.splitn(2, '/').collect();
    if parts.len() == 2 {
        let num: f64 = parts[0].parse().unwrap_or(0.0);
        let den: f64 = parts[1].parse().unwrap_or(1.0);
        if den != 0.0 { num / den } else { 0.0 }
    } else { s.parse().unwrap_or(0.0) }
}

fn parse_str_f64(v: &serde_json::Value, key: &str) -> f64 {
    v.get(key).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0)
}

fn parse_str_u64(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn run_ffprobe_metadata(path: &Path) -> Option<serde_json::Value> {
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_streams", "-show_format"])
        .arg(path).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

fn run_ffprobe_frames(path: &Path, sample_n: usize) -> Option<serde_json::Value> {
    let interval = format!("%+#{}", sample_n);
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_frames",
               "-select_streams", "v:0", "-read_intervals", &interval])
        .arg(path).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    if out.stdout.is_empty() { return None; }
    serde_json::from_slice(&out.stdout).ok()
}

fn extract_video_info(path: &Path, meta: &serde_json::Value) -> Option<VideoInfo> {
    let streams = meta.get("streams")?.as_array()?;
    let format  = meta.get("format")?;

    let format_name     = format.get("format_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let format_duration = parse_str_f64(format, "duration");
    let format_bit_rate = parse_str_u64(format, "bit_rate");

    let vstream = streams.iter().find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("video"))?;

    let codec_name     = vstream.get("codec_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let codec_profile  = vstream.get("profile").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let width          = vstream.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let height         = vstream.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let avg_frame_rate = vstream.get("avg_frame_rate").and_then(|v| v.as_str()).map(parse_rational).unwrap_or(0.0);
    let pix_fmt        = vstream.get("pix_fmt").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let stream_duration  = parse_str_f64(vstream, "duration");
    let video_bit_rate   = parse_str_u64(vstream, "bit_rate");
    let video_start_time = parse_str_f64(vstream, "start_time");

    let rotation = {
        let mut rot = 0i32;
        if let Some(side_data) = vstream.get("side_data_list").and_then(|v| v.as_array()) {
            for sd in side_data {
                if let Some(r) = sd.get("rotation").and_then(|v| v.as_i64()) { rot = r as i32; break; }
            }
        }
        if rot == 0 {
            if let Some(tags) = vstream.get("tags") {
                if let Some(r) = tags.get("rotate").and_then(|v| v.as_str()) { rot = r.parse().unwrap_or(0); }
            }
        }
        rot
    };

    let astream = streams.iter().find(|s| s.get("codec_type").and_then(|v| v.as_str()) == Some("audio"));
    let has_audio = astream.is_some();
    let audio_start_time = astream.map(|s| parse_str_f64(s, "start_time")).unwrap_or(0.0);

    Some(VideoInfo {
        path: path.to_path_buf(), format_name, format_duration, format_bit_rate,
        codec_name, codec_profile, width, height, avg_frame_rate,
        stream_duration, video_bit_rate, pix_fmt, rotation, video_start_time,
        has_audio, audio_start_time,
        frame_pts: None, frame_pkt_sizes: None, keyframe_indices: None,
    })
}

fn merge_frame_data(info: &mut VideoInfo, frames_json: &serde_json::Value) {
    let frames = match frames_json.get("frames").and_then(|v| v.as_array()) {
        Some(f) => f, None => return,
    };
    if frames.is_empty() { return; }

    let mut pts_vec = Vec::with_capacity(frames.len());
    let mut size_vec = Vec::with_capacity(frames.len());
    let mut kf_indices = Vec::new();

    for (i, frame) in frames.iter().enumerate() {
        let pts = frame.get("pkt_pts_time").and_then(|v| v.as_str()).and_then(|s| s.parse::<f64>().ok())
            .or_else(|| frame.get("best_effort_timestamp_time").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()))
            .unwrap_or(-1.0);
        let pkt_size = frame.get("pkt_size").and_then(|v| v.as_str()).and_then(|s| s.parse::<u64>().ok())
            .or_else(|| frame.get("pkt_size").and_then(|v| v.as_u64())).unwrap_or(0);
        let is_key = frame.get("key_frame").and_then(|v| v.as_u64()).unwrap_or(0) == 1;
        pts_vec.push(pts);
        size_vec.push(pkt_size);
        if is_key { kf_indices.push(i); }
    }

    info.frame_pts = Some(pts_vec);
    info.frame_pkt_sizes = Some(size_vec);
    info.keyframe_indices = Some(kf_indices);
}

fn check_vfr(info: &VideoInfo) -> bool {
    let pts = match &info.frame_pts { Some(p) if p.len() >= 10 => p, _ => return false };
    let valid: Vec<f64> = pts.windows(2).filter_map(|w| { let d = w[1]-w[0]; if d>0.0 { Some(d) } else { None } }).collect();
    if valid.len() < 8 { return false; }
    let m = mean(&valid);
    if m <= 0.0 { return false; }
    stddev(&valid) / m > 0.15
}

fn check_frame_drop(info: &VideoInfo) -> bool {
    let pts = match &info.frame_pts { Some(p) if p.len() >= 5 => p, _ => return false };
    if info.avg_frame_rate <= 0.0 { return false; }
    let expected = 1.0 / info.avg_frame_rate;
    let threshold = expected * 2.5;
    let mut drop_count = 0usize;
    for w in pts.windows(2) {
        let d = w[1] - w[0];
        if d > threshold { drop_count += (d / expected).floor() as usize - 1; }
    }
    drop_count >= 3
}

fn check_gop(info: &VideoInfo) -> (bool, bool) {
    let pts = match &info.frame_pts { Some(p) => p, None => return (false, false) };
    let kf_idx = match &info.keyframe_indices { Some(k) if k.len() >= 2 => k, _ => return (false, false) };
    let kf_times: Vec<f64> = kf_idx.iter().filter_map(|&i| pts.get(i).copied()).filter(|&t| t >= 0.0).collect();
    if kf_times.len() < 2 { return (false, false); }
    let intervals: Vec<f64> = kf_times.windows(2).map(|w| w[1]-w[0]).filter(|&d| d > 0.0).collect();
    if intervals.is_empty() { return (false, false); }
    let too_long = intervals.iter().any(|&d| d > 10.0);
    let irregular = if intervals.len() >= 3 { let m = mean(&intervals); if m > 0.0 { stddev(&intervals)/m > 0.5 } else { false } } else { false };
    (too_long, irregular)
}

fn check_bitrate_spike(info: &VideoInfo) -> bool {
    let sizes = match &info.frame_pkt_sizes { Some(s) if s.len() >= 20 => s, _ => return false };
    let float_sizes: Vec<f64> = sizes.iter().map(|&s| s as f64).collect();
    let m = mean(&float_sizes);
    if m <= 0.0 { return false; }
    let threshold = m + 4.0 * stddev(&float_sizes);
    float_sizes.iter().filter(|&&s| s > threshold).count() >= 2
}

fn check_codec_compat(info: &VideoInfo) -> bool {
    let codec = info.codec_name.to_lowercase();
    let profile = info.codec_profile.to_lowercase();
    let pix = info.pix_fmt.to_lowercase();
    if codec == "hevc" && pix.contains("10") { return true; }
    if codec == "hevc" && info.width >= 3840 { return true; }
    if codec == "av1" { return true; }
    if codec == "vp9" { let fmt = info.format_name.to_lowercase(); if !fmt.contains("matroska") && !fmt.contains("webm") { return true; } }
    if profile.contains("4:4:4") || profile.contains("high 10") { return true; }
    if (info.width as u64) * (info.height as u64) > 3840 * 2160 { return true; }
    false
}

fn check_av_sync(info: &VideoInfo) -> bool {
    if !info.has_audio { return false; }
    (info.audio_start_time - info.video_start_time).abs() > 0.200
}

fn check_corrupt_frame(info: &VideoInfo) -> bool {
    let pts   = match &info.frame_pts        { Some(p) => p, None => return false };
    let sizes = match &info.frame_pkt_sizes  { Some(s) => s, None => return false };
    if sizes.iter().filter(|&&s| s == 0).count() >= 1 { return true; }
    let mut non_mono = 0usize;
    for w in pts.windows(2) { if w[0] >= 0.0 && w[1] >= 0.0 && w[1] <= w[0] { non_mono += 1; } }
    non_mono >= 2
}

fn check_container_mismatch(info: &VideoInfo) -> bool {
    let fmt = info.format_name.to_lowercase(); let codec = info.codec_name.to_lowercase();
    (fmt.contains("avi") && (codec == "hevc" || codec == "av1")) ||
    (fmt.contains("flv") && codec == "hevc") ||
    (fmt.contains("mp4") && (codec == "vp8" || codec == "vp9"))
}

fn check_low_bitrate(info: &VideoInfo) -> bool {
    let br = if info.video_bit_rate > 0 { info.video_bit_rate } else { info.format_bit_rate };
    if br == 0 { return false; }
    let min = if info.width >= 3840 { 8_000_000u64 } else if info.width >= 1920 { 2_000_000 }
        else if info.width >= 1280 { 800_000 } else if info.width >= 854 { 400_000 } else { 150_000 };
    br < min
}

fn check_high_bitrate(info: &VideoInfo) -> bool {
    let br = if info.video_bit_rate > 0 { info.video_bit_rate } else { info.format_bit_rate };
    if br == 0 { return false; }
    let max = if info.width >= 3840 { 80_000_000u64 } else if info.width >= 1920 { 20_000_000 }
        else if info.width >= 1280 { 10_000_000 } else { 5_000_000 };
    br > max
}

fn check_unusual_resolution(info: &VideoInfo) -> bool {
    if info.width == 0 || info.height == 0 { return false; }
    const SW: &[u32] = &[7680,3840,2560,1920,1280,1024,854,640,426,320];
    const SH: &[u32] = &[4320,2160,1440,1080,720,576,480,360,240,180];
    !SW.contains(&info.width) && !SH.contains(&info.height)
}

fn check_abnormal_duration(info: &VideoInfo) -> bool {
    if info.format_duration <= 0.0 || info.stream_duration <= 0.0 { return false; }
    (info.format_duration - info.stream_duration).abs() / info.format_duration > 0.05
}

fn analyze_video(info: &VideoInfo) -> Vec<IssueKind> {
    let mut issues = Vec::new();
    if check_vfr(info)               { issues.push(IssueKind::Vfr); }
    if check_frame_drop(info)        { issues.push(IssueKind::FrameDrop); }
    if check_corrupt_frame(info)     { issues.push(IssueKind::CorruptFrame); }
    if check_codec_compat(info)      { issues.push(IssueKind::CodecCompat); }
    if check_av_sync(info)           { issues.push(IssueKind::AvSync); }
    if check_bitrate_spike(info)     { issues.push(IssueKind::BitrateSpike); }
    let (gop_long, gop_irr) = check_gop(info);
    if gop_long { issues.push(IssueKind::GopTooLong); }
    if gop_irr  { issues.push(IssueKind::GopIrregular); }
    if check_low_bitrate(info)        { issues.push(IssueKind::LowBitrate); }
    if check_container_mismatch(info) { issues.push(IssueKind::ContainerMismatch); }
    if !info.has_audio                { issues.push(IssueKind::NoAudio); }
    if check_abnormal_duration(info)  { issues.push(IssueKind::AbnormalDuration); }
    if check_high_bitrate(info)       { issues.push(IssueKind::HighBitrate); }
    if check_unusual_resolution(info) { issues.push(IssueKind::UnusualResolution); }
    if info.rotation != 0             { issues.push(IssueKind::RotationTag); }
    issues
}

fn compute_severity(issues: &[IssueKind]) -> u32 {
    issues.iter().map(|i| i.score()).sum::<u32>().min(100)
}

fn categorize(severity: u32) -> ResultCategory {
    match severity { 0 => ResultCategory::Normal, 1..=29 => ResultCategory::Warning, _ => ResultCategory::Problem }
}

fn collect_video_files(dir: &Path, results: &mut Vec<PathBuf>, cancel: &Arc<Mutex<bool>>, min_size_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *cancel.lock().unwrap() { return; }
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if path.is_dir() {
            if !name.contains("recycle") { collect_video_files(&path, results, cancel, min_size_bytes); }
        } else {
            let ext = path.extension().unwrap_or_default().to_string_lossy().to_lowercase();
            if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
                if let Ok(meta) = entry.metadata() {
                    if meta.len() >= min_size_bytes { results.push(path); }
                }
            }
        }
    }
}

struct VidScanResult {
    results: Vec<AnalysisResult>,
    total: usize,
    elapsed_secs: f64,
    cancelled: bool,
}

fn run_vid_scan(
    folders: Vec<PathBuf>, cancel: Arc<Mutex<bool>>,
    sample_frames: usize, min_size_bytes: u64, hwnd_raw: usize,
) -> VidScanResult {
    let start = Instant::now();
    let mut video_files = Vec::new();
    for folder in &folders {
        collect_video_files(folder, &mut video_files, &cancel, min_size_bytes);
        if *cancel.lock().unwrap() {
            return VidScanResult { results: vec![], total: 0, elapsed_secs: start.elapsed().as_secs_f64(), cancelled: true };
        }
    }

    let total = video_files.len();
    let mut results = Vec::with_capacity(total);

    for (i, path) in video_files.iter().enumerate() {
        if *cancel.lock().unwrap() { break; }
        unsafe {
            PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_SCAN_PROGRESS, WPARAM(i+1), LPARAM(total as isize)).ok();
        }
        let meta_json = match run_ffprobe_metadata(path) { Some(j) => j, None => continue };
        let mut info = match extract_video_info(path, &meta_json) { Some(i) => i, None => continue };
        if !*cancel.lock().unwrap() {
            if let Some(frames_json) = run_ffprobe_frames(path, sample_frames) {
                merge_frame_data(&mut info, &frames_json);
            }
        }
        let issues = analyze_video(&info);
        let severity = compute_severity(&issues);
        let category = categorize(severity);
        results.push(AnalysisResult { video: info, issues, severity, category });
    }

    VidScanResult { results, total, elapsed_secs: start.elapsed().as_secs_f64(), cancelled: *cancel.lock().unwrap() }
}

// ─────────────────────────────────────────────────────────────────────────────
// 앱 상태
// ─────────────────────────────────────────────────────────────────────────────

struct FileFinderState {
    results: Vec<PathBuf>,
    cancel: Arc<Mutex<bool>>,
}

struct DupFinderState {
    groups: Vec<DupGroup>,
    result_labels: Vec<String>,
    result_paths: Vec<Option<PathBuf>>,
    selected: std::collections::HashSet<PathBuf>,
    cancel: Arc<Mutex<bool>>,
}

impl DupFinderState {
    fn rebuild_results(&mut self) {
        self.result_labels.clear();
        self.result_paths.clear();
        for (i, g) in self.groups.iter().enumerate() {
            self.result_labels.push(format!("  ▼ Group {}  ({} files, {})", i+1, g.files.len(), format_size(g.size)));
            self.result_paths.push(None);
            for f in &g.files {
                let selected = self.selected.contains(&f.path);
                let prefix = if selected { "  [v] " } else { "  [ ] " };
                self.result_labels.push(format!("{}{}", prefix, f.path.display()));
                self.result_paths.push(Some(f.path.clone()));
            }
        }
    }
}

struct VideoCheckerState {
    results: Vec<AnalysisResult>,
    result_labels: Vec<String>,
    result_paths: Vec<Option<PathBuf>>,
    cancel: Arc<Mutex<bool>>,
    sample_frames: usize,
    min_file_size_mb: u64,
}

impl VideoCheckerState {
    fn rebuild_results(&mut self) {
        self.result_labels.clear();
        self.result_paths.clear();
        let mut problems: Vec<&AnalysisResult> = self.results.iter().filter(|r| r.category == ResultCategory::Problem).collect();
        let mut warnings: Vec<&AnalysisResult> = self.results.iter().filter(|r| r.category == ResultCategory::Warning).collect();
        let normals: Vec<&AnalysisResult> = self.results.iter().filter(|r| r.category == ResultCategory::Normal).collect();
        problems.sort_by(|a, b| b.severity.cmp(&a.severity));
        warnings.sort_by(|a, b| b.severity.cmp(&a.severity));
        if !problems.is_empty() {
            self.result_labels.push(format!("  ── Problem  ({} files) ──────────────────────────", problems.len()));
            self.result_paths.push(None);
            for r in &problems { self.result_labels.push(r.label()); self.result_paths.push(Some(r.video.path.clone())); }
        }
        if !warnings.is_empty() {
            self.result_labels.push(format!("  ── Warning  ({} files) ──────────────────────────", warnings.len()));
            self.result_paths.push(None);
            for r in &warnings { self.result_labels.push(r.label()); self.result_paths.push(Some(r.video.path.clone())); }
        }
        if !normals.is_empty() {
            self.result_labels.push(format!("  ── OK  ({} files) ───────────────────────────────", normals.len()));
            self.result_paths.push(None);
            for r in &normals { self.result_labels.push(r.label()); self.result_paths.push(Some(r.video.path.clone())); }
        }
    }
}

struct AppState {
    current_tab: Tab,
    folders: Vec<PathBuf>,
    file_finder: FileFinderState,
    dup_finder: DupFinderState,
    video_checker: VideoCheckerState,
}

impl AppState {
    fn new() -> Self {
        Self {
            current_tab: Tab::FileFinder,
            folders: vec![],
            file_finder: FileFinderState {
                results: vec![],
                cancel: Arc::new(Mutex::new(false)),
            },
            dup_finder: DupFinderState {
                groups: vec![],
                result_labels: vec![],
                result_paths: vec![],
                selected: Default::default(),
                cancel: Arc::new(Mutex::new(false)),
            },
            video_checker: VideoCheckerState {
                results: vec![],
                result_labels: vec![],
                result_paths: vec![],
                cancel: Arc::new(Mutex::new(false)),
                sample_frames: 500,
                min_file_size_mb: 1,
            },
        }
    }

    fn current_folders(&self) -> &Vec<PathBuf> {
        &self.folders
    }

    fn current_folders_mut(&mut self) -> &mut Vec<PathBuf> {
        &mut self.folders
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 유틸
// ─────────────────────────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes < 1024 { format!("{} B", bytes) }
    else if bytes < 1024*1024 { format!("{:.1} KB", bytes as f64 / 1024.0) }
    else if bytes < 1024*1024*1024 { format!("{:.1} MB", bytes as f64 / (1024.0*1024.0)) }
    else { format!("{:.2} GB", bytes as f64 / (1024.0*1024.0*1024.0)) }
}

fn format_duration(secs: f64) -> String {
    if secs < 1.0 { format!("{:.0}ms", secs * 1000.0) }
    else if secs < 60.0 { format!("{:.1}s", secs) }
    else { format!("{:.0}m {:.0}s", secs / 60.0, secs % 60.0) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Win32 전역
// ─────────────────────────────────────────────────────────────────────────────

static mut HWND_MAIN: HWND = HWND(std::ptr::null_mut());

// 탭 버튼
static mut HWND_TAB_FILE:  HWND = HWND(std::ptr::null_mut());
static mut HWND_TAB_DUP:   HWND = HWND(std::ptr::null_mut());
static mut HWND_TAB_VIDEO: HWND = HWND(std::ptr::null_mut());

// 공유 컨트롤
static mut HWND_FOLDER_LIST: HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_ADD:     HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_REMOVE:  HWND = HWND(std::ptr::null_mut());
static mut HWND_RESULT_LIST: HWND = HWND(std::ptr::null_mut());
static mut HWND_STATUS:      HWND = HWND(std::ptr::null_mut());
static mut HWND_STATS:       HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_CANCEL:  HWND = HWND(std::ptr::null_mut());
static mut HWND_LBL_FOLDER:  HWND = HWND(std::ptr::null_mut());
static mut HWND_LBL_RESULT:  HWND = HWND(std::ptr::null_mut());

// 파일 검색 전용
static mut HWND_EDIT_QUERY:  HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_SEARCH:  HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_OPEN:    HWND = HWND(std::ptr::null_mut());
static mut HWND_LBL_QUERY:   HWND = HWND(std::ptr::null_mut());

// 중복 검색 전용
static mut HWND_BTN_SCAN_DUP: HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_DELETE:   HWND = HWND(std::ptr::null_mut());

// 영상 체크 전용
static mut HWND_BTN_SCAN_VID: HWND = HWND(std::ptr::null_mut());
static mut HWND_EDIT_FRAMES:  HWND = HWND(std::ptr::null_mut());
static mut HWND_EDIT_SIZE:    HWND = HWND(std::ptr::null_mut());
static mut HWND_DETAIL:       HWND = HWND(std::ptr::null_mut());
static mut HWND_BTN_OPEN_VID: HWND = HWND(std::ptr::null_mut());
static mut HWND_LBL_FRAMES:   HWND = HWND(std::ptr::null_mut());
static mut HWND_LBL_SIZE:     HWND = HWND(std::ptr::null_mut());

// EDIT 서브클래스 원본 프로시저
static mut EDIT_QUERY_ORIG_PROC: Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT> = None;

// 폰트 & 브러시
static mut H_FONT:        HFONT  = HFONT(std::ptr::null_mut());
static mut H_FONT_LARGE:  HFONT  = HFONT(std::ptr::null_mut());
static mut H_FONT_TAB:    HFONT  = HFONT(std::ptr::null_mut());
static mut BRUSH_BG:      HBRUSH = HBRUSH(std::ptr::null_mut());
static mut BRUSH_SURFACE: HBRUSH = HBRUSH(std::ptr::null_mut());
static mut BRUSH_BTN:     HBRUSH = HBRUSH(std::ptr::null_mut());
static mut BRUSH_EDIT:    HBRUSH = HBRUSH(std::ptr::null_mut());
static mut BRUSH_TAB_ACT: HBRUSH = HBRUSH(std::ptr::null_mut());

// 스캔 결과 임시 저장 (스레드 → 메인)
static mut LAST_TOTAL:     usize = 0;
static mut LAST_ELAPSED:   f64   = 0.0;
static mut LAST_CANCELLED: bool  = false;

static APP_STATE: std::sync::OnceLock<Mutex<AppState>> = std::sync::OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, AppState> {
    APP_STATE.get().unwrap().lock().unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────
// Win32 유틸
// ─────────────────────────────────────────────────────────────────────────────

fn wstr(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn send_msg(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> LRESULT {
    SendMessageW(hwnd, msg, Some(WPARAM(wp)), Some(LPARAM(lp)))
}

unsafe fn set_text(hwnd: HWND, text: &str) {
    SetWindowTextW(hwnd, PCWSTR(wstr(text).as_ptr())).ok();
}

unsafe fn get_edit_text(hwnd: HWND) -> String {
    let len = SendMessageW(hwnd, WM_GETTEXTLENGTH, Some(WPARAM(0)), Some(LPARAM(0))).0 as usize;
    if len == 0 { return String::new(); }
    let mut buf = vec![0u16; len + 1];
    SendMessageW(hwnd, WM_GETTEXT, Some(WPARAM(buf.len())), Some(LPARAM(buf.as_mut_ptr() as isize)));
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn create_control_font(
    class: &str, text: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE,
    x: i32, y: i32, w: i32, h: i32, parent: HWND, id: u16, font: HFONT,
) -> HWND {
    let hinstance: HINSTANCE = GetModuleHandleW(PCWSTR::null()).unwrap().into();
    let hwnd = CreateWindowExW(ex, PCWSTR(wstr(class).as_ptr()), PCWSTR(wstr(text).as_ptr()),
        style, x, y, w, h, Some(parent), Some(HMENU(id as isize as *mut _)), Some(hinstance), None).unwrap();
    send_msg(hwnd, WM_SETFONT, font.0 as usize, 1);
    hwnd
}

unsafe fn create_control(class: &str, text: &str, style: WINDOW_STYLE, ex: WINDOW_EX_STYLE,
    x: i32, y: i32, w: i32, h: i32, parent: HWND, id: u16) -> HWND {
    create_control_font(class, text, style, ex, x, y, w, h, parent, id, H_FONT)
}

unsafe fn create_label(parent: HWND, text: &str, font: HFONT) -> HWND {
    let hinstance: HINSTANCE = GetModuleHandleW(PCWSTR::null()).unwrap().into();
    let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), PCWSTR(wstr("STATIC").as_ptr()),
        PCWSTR(wstr(text).as_ptr()), WS_CHILD | WS_VISIBLE, 0,0,0,0,
        Some(parent), None, Some(hinstance), None).unwrap();
    send_msg(hwnd, WM_SETFONT, font.0 as usize, 1);
    hwnd
}

unsafe fn pick_folder(parent: HWND) -> Option<PathBuf> {
    let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
    let mut opts = dialog.GetOptions().ok()?;
    opts |= FOS_PICKFOLDERS;
    dialog.SetOptions(opts).ok()?;
    let title = wstr("폴더 선택");
    dialog.SetTitle(PCWSTR(title.as_ptr())).ok()?;
    if dialog.Show(Some(parent)).is_err() { return None; }
    let item = dialog.GetResult().ok()?;
    let path_ptr = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    Some(PathBuf::from(path_ptr.to_string().ok()?))
}

unsafe fn open_in_explorer(path: &Path) {
    let arg = wstr(&format!("/select,\"{}\"", path.display()));
    let exe = wstr("explorer.exe");
    ShellExecuteW(Some(HWND_MAIN), PCWSTR(wstr("open").as_ptr()),
        PCWSTR(exe.as_ptr()), PCWSTR(arg.as_ptr()), PCWSTR::null(), SW_SHOW);
}

// ─────────────────────────────────────────────────────────────────────────────
// 탭 전환 UI
// ─────────────────────────────────────────────────────────────────────────────

unsafe fn show_tab(tab: Tab) {
    // 현재 탭을 상태에 기록 후 즉시 drop
    {
        let mut st = state();
        st.current_tab = tab;
    }

    let is_file  = tab == Tab::FileFinder;
    let is_dup   = tab == Tab::DuplicateFinder;
    let is_video = tab == Tab::VideoChecker;

    let show_hide = |hwnd: HWND, visible: bool| {
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE }).ok();
    };

    // 파일 검색 전용
    show_hide(HWND_EDIT_QUERY,  is_file);
    show_hide(HWND_BTN_SEARCH,  is_file);
    show_hide(HWND_BTN_OPEN,    is_file);
    show_hide(HWND_LBL_QUERY,   is_file);

    // 중복 검색 전용
    show_hide(HWND_BTN_SCAN_DUP, is_dup);
    show_hide(HWND_BTN_DELETE,   is_dup);

    // 영상 체크 전용
    show_hide(HWND_BTN_SCAN_VID, is_video);
    show_hide(HWND_EDIT_FRAMES,  is_video);
    show_hide(HWND_EDIT_SIZE,    is_video);
    show_hide(HWND_DETAIL,       is_video);
    show_hide(HWND_BTN_OPEN_VID, is_video);
    show_hide(HWND_LBL_FRAMES,   is_video);
    show_hide(HWND_LBL_SIZE,     is_video);

    // 결과 라벨 텍스트 업데이트
    let result_label = match tab {
        Tab::FileFinder      => "검색 결과",
        Tab::DuplicateFinder => "중복 파일",
        Tab::VideoChecker    => "분석 결과",
    };
    set_text(HWND_LBL_RESULT, result_label);

    // 폴더 목록 새로고침 (탭마다 독립적)
    refresh_folder_list();
    refresh_result_list();

    // 통계 힌트 텍스트
    let hint = match tab {
        Tab::FileFinder      => "폴더를 추가하고 검색어를 입력하세요.",
        Tab::DuplicateFinder => "폴더를 추가하고 스캔을 시작하세요.",
        Tab::VideoChecker    => "폴더를 추가하고 스캔을 시작하세요.",
    };
    let has_stats = {
        let st = state();
        match tab {
            Tab::FileFinder      => !st.file_finder.results.is_empty(),
            Tab::DuplicateFinder => !st.dup_finder.groups.is_empty() || !st.dup_finder.result_labels.is_empty(),
            Tab::VideoChecker    => !st.video_checker.results.is_empty(),
        }
    };
    if !has_stats {
        set_text(HWND_STATS, hint);
        set_text(HWND_STATUS, "");
    }

    // 레이아웃 재계산
    layout(HWND_MAIN);
}

unsafe fn refresh_folder_list() {
    send_msg(HWND_FOLDER_LIST, LB_RESETCONTENT, 0, 0);
    let st = state();
    for f in st.current_folders() {
        let s = wstr(&f.display().to_string());
        send_msg(HWND_FOLDER_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
    }
}

unsafe fn refresh_result_list() {
    send_msg(HWND_RESULT_LIST, LB_RESETCONTENT, 0, 0);
    let st = state();
    match st.current_tab {
        Tab::FileFinder => {
            for path in &st.file_finder.results {
                let s = wstr(&path.display().to_string());
                send_msg(HWND_RESULT_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
            }
        }
        Tab::DuplicateFinder => {
            for label in &st.dup_finder.result_labels {
                let s = wstr(label);
                send_msg(HWND_RESULT_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
            }
        }
        Tab::VideoChecker => {
            for label in &st.video_checker.result_labels {
                let s = wstr(label);
                send_msg(HWND_RESULT_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 레이아웃
// ─────────────────────────────────────────────────────────────────────────────

unsafe fn layout(hwnd: HWND) {
    let mut rc = RECT::default();
    GetClientRect(hwnd, &mut rc).ok();
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;

    let m       = 12;
    let gap     = 8;
    let tab_h   = 44;   // 탭 버튼 높이
    let tab_gap = 4;    // 탭 버튼 간격
    let label_h = 28;
    let btn_h   = 36;
    let btn_w   = 120;
    let status_h = 26;

    // 탭 버튼 (상단)
    let tab_count = 3;
    let tab_btn_w = (w - m * 2 - tab_gap * (tab_count - 1)) / tab_count;
    MoveWindow(HWND_TAB_FILE,  m,                                   m, tab_btn_w, tab_h, true).ok();
    MoveWindow(HWND_TAB_DUP,   m + tab_btn_w + tab_gap,             m, tab_btn_w, tab_h, true).ok();
    MoveWindow(HWND_TAB_VIDEO, m + (tab_btn_w + tab_gap) * 2,       m, tab_btn_w, tab_h, true).ok();

    let content_top = m + tab_h + gap;
    let tab = { state().current_tab };

    // 탭별 중간 영역 높이 계산 (폴더 목록 아래 ~ 결과 라벨 위)
    let mid_h: i32 = match tab {
        Tab::FileFinder      => label_h + gap + btn_h + gap, // 검색어 라벨 + 입력행
        Tab::DuplicateFinder => btn_h + gap,
        Tab::VideoChecker    => btn_h + gap + btn_h + gap,   // 옵션 행 + 스캔 버튼 행
    };

    // 하단 고정 높이: 결과 라벨 + 결과 리스트 아래 요소들 + 하단 여백
    let below_result: i32 = match tab {
        Tab::VideoChecker => 80 + gap + status_h + gap + status_h + m, // detail + status + stats
        _                 =>            status_h + gap + status_h + m,
    };

    // 폴더 영역 비율 고정: 폴더 2, 결과 5
    let folder_area_h = h - content_top - mid_h - label_h - gap - below_result;
    let folder_h = folder_area_h * 2 / 7;
    let result_h = folder_area_h - folder_h;

    let folder_w = w - btn_w - m * 3;
    let btn_x    = w - btn_w - m;

    // 폴더 라벨 + 목록
    let y_lbl_folder = content_top;
    let y_folder     = y_lbl_folder + label_h + gap;
    MoveWindow(HWND_LBL_FOLDER,  m,     y_lbl_folder, w - m*2,  label_h,  true).ok();
    MoveWindow(HWND_FOLDER_LIST, m,     y_folder,     folder_w, folder_h.max(30), true).ok();
    MoveWindow(HWND_BTN_ADD,     btn_x, y_folder,               btn_w, btn_h, true).ok();
    MoveWindow(HWND_BTN_REMOVE,  btn_x, y_folder + btn_h + gap, btn_w, btn_h, true).ok();

    let y_mid = y_folder + folder_h.max(30) + gap;

    // result_h 최소 보장
    let result_h = result_h.max(30);

    match tab {
        Tab::FileFinder => {
            let lbl_q_w = w - m * 2;
            MoveWindow(HWND_LBL_QUERY, m, y_mid, lbl_q_w, label_h, true).ok();
            let y_query = y_mid + label_h + gap;
            let search_w = 100;
            let cancel_w = 80;
            let edit_w = w - search_w - cancel_w - m*2 - gap*2;
            MoveWindow(HWND_EDIT_QUERY,  m,                                    y_query, edit_w,   btn_h, true).ok();
            MoveWindow(HWND_BTN_SEARCH,  m + edit_w + gap,                     y_query, search_w, btn_h, true).ok();
            MoveWindow(HWND_BTN_CANCEL,  m + edit_w + gap + search_w + gap,    y_query, cancel_w, btn_h, true).ok();

            let y_lbl_result = y_query + btn_h + gap;
            let open_w = 150;
            MoveWindow(HWND_LBL_RESULT, m, y_lbl_result, w - open_w - m*2 - gap, label_h, true).ok();
            MoveWindow(HWND_BTN_OPEN,   w - open_w - m, y_lbl_result, open_w, label_h, true).ok();
            let y_result = y_lbl_result + label_h + gap;
            MoveWindow(HWND_RESULT_LIST, m, y_result, w - m*2, result_h, true).ok();
            let y_status = y_result + result_h + gap;
            MoveWindow(HWND_STATUS, m, y_status,                 w - m*2, status_h, true).ok();
            MoveWindow(HWND_STATS,  m, y_status + status_h + gap, w - m*2, status_h, true).ok();
        }
        Tab::DuplicateFinder => {
            MoveWindow(HWND_BTN_SCAN_DUP, m,                           y_mid, 120, btn_h, true).ok();
            MoveWindow(HWND_BTN_CANCEL,   m + 120 + gap,               y_mid,  90, btn_h, true).ok();
            MoveWindow(HWND_BTN_DELETE,   m + 120 + gap + 90 + gap,    y_mid, 130, btn_h, true).ok();

            let y_lbl_result = y_mid + btn_h + gap;
            MoveWindow(HWND_LBL_RESULT, m, y_lbl_result, w - m*2, label_h, true).ok();
            let y_result = y_lbl_result + label_h + gap;
            MoveWindow(HWND_RESULT_LIST, m, y_result, w - m*2, result_h, true).ok();
            let y_status = y_result + result_h + gap;
            MoveWindow(HWND_STATUS, m, y_status,                 w - m*2, status_h, true).ok();
            MoveWindow(HWND_STATS,  m, y_status + status_h + gap, w - m*2, status_h, true).ok();
        }
        Tab::VideoChecker => {
            let lbl_small_w = 110;
            let edit_w_small = 60;
            let mut ox = m;
            MoveWindow(HWND_LBL_FRAMES,  ox, y_mid + 4, lbl_small_w,      label_h, true).ok();
            ox += lbl_small_w + 4;
            MoveWindow(HWND_EDIT_FRAMES, ox, y_mid,      edit_w_small,     btn_h,   true).ok();
            ox += edit_w_small + gap * 2;
            MoveWindow(HWND_LBL_SIZE,    ox, y_mid + 4, lbl_small_w + 10, label_h, true).ok();
            ox += lbl_small_w + 14;
            MoveWindow(HWND_EDIT_SIZE,   ox, y_mid,      edit_w_small,     btn_h,   true).ok();

            let y_btns = y_mid + btn_h + gap;
            MoveWindow(HWND_BTN_SCAN_VID, m,             y_btns, 120, btn_h, true).ok();
            MoveWindow(HWND_BTN_CANCEL,   m + 120 + gap, y_btns,  90, btn_h, true).ok();

            let y_lbl_result = y_btns + btn_h + gap;
            let open_w = 150;
            MoveWindow(HWND_LBL_RESULT,   m,             y_lbl_result, w - open_w - m*2 - gap, label_h, true).ok();
            MoveWindow(HWND_BTN_OPEN_VID, w - open_w - m, y_lbl_result, open_w, label_h, true).ok();
            let y_result = y_lbl_result + label_h + gap;
            MoveWindow(HWND_RESULT_LIST, m, y_result, w - m*2, result_h, true).ok();
            let y_detail = y_result + result_h + gap;
            MoveWindow(HWND_DETAIL, m, y_detail, w - m*2, 80, true).ok();
            let y_status = y_detail + 80 + gap;
            MoveWindow(HWND_STATUS, m, y_status,                 w - m*2, status_h, true).ok();
            MoveWindow(HWND_STATS,  m, y_status + status_h + gap, w - m*2, status_h, true).ok();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 이벤트 핸들러
// ─────────────────────────────────────────────────────────────────────────────

unsafe fn on_add_folder(hwnd: HWND) {
    if let Some(path) = pick_folder(hwnd) {
        let mut st = state();
        if !st.current_folders().contains(&path) {
            st.current_folders_mut().push(path);
            drop(st);
            refresh_folder_list();
        }
    }
}

unsafe fn on_remove_folder() {
    let idx = send_msg(HWND_FOLDER_LIST, LB_GETCURSEL, 0, 0).0 as usize;
    let mut st = state();
    if idx < st.current_folders().len() {
        st.current_folders_mut().remove(idx);
        drop(st);
        refresh_folder_list();
    }
}

// ── 파일 검색 ──────────────────────────────────────────────────────────────

unsafe fn on_file_search(hwnd: HWND) {
    let keyword = get_edit_text(HWND_EDIT_QUERY).trim().to_string();
    if keyword.is_empty() { set_text(HWND_STATS, "검색어를 입력하세요."); return; }

    let folders = {
        let st = state();
        if st.folders.is_empty() { set_text(HWND_STATS, "폴더를 추가하세요."); return; }
        st.folders.clone()
    };

    {
        let mut st = state();
        *st.file_finder.cancel.lock().unwrap() = false;
        st.file_finder.results.clear();
    }

    refresh_result_list();
    _ = EnableWindow(HWND_BTN_SEARCH, false);
    _ = EnableWindow(HWND_BTN_ADD,    false);
    _ = EnableWindow(HWND_BTN_REMOVE, false);
    _ = EnableWindow(HWND_BTN_CANCEL, true);
    _ = EnableWindow(HWND_BTN_OPEN,   false);
    set_text(HWND_STATS, &format!("\"{}\" 검색 중...", keyword));
    set_text(HWND_STATUS, "");

    let cancel   = state().file_finder.cancel.clone();
    let hwnd_raw = hwnd.0 as usize;

    std::thread::spawn(move || {
        let result = run_file_search(folders, keyword, cancel);
        unsafe {
            { let mut st = state(); st.file_finder.results = result.files; }
            LAST_TOTAL     = result.total_searched;
            LAST_ELAPSED   = result.elapsed_secs;
            LAST_CANCELLED = result.cancelled;
            PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_SEARCH_DONE, WPARAM(0), LPARAM(0)).ok();
        }
    });
}

unsafe fn on_file_search_done(_hwnd: HWND) {
    let count     = state().file_finder.results.len();
    let total     = LAST_TOTAL;
    let elapsed   = LAST_ELAPSED;
    let cancelled = LAST_CANCELLED;

    let stats = format!("결과: {}개 | 검색된 파일: {} · {}", count, total, format_duration(elapsed));
    let stats = if cancelled { format!("[취소됨] {}", stats) } else { stats };
    set_text(HWND_STATS, &stats);
    set_text(HWND_STATUS, "");

    refresh_result_list();

    if count == 0 {
        let s = wstr("일치하는 파일이 없습니다.");
        send_msg(HWND_RESULT_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
    }

    _ = EnableWindow(HWND_BTN_SEARCH, true);
    _ = EnableWindow(HWND_BTN_ADD,    true);
    _ = EnableWindow(HWND_BTN_REMOVE, true);
    _ = EnableWindow(HWND_BTN_CANCEL, false);
    if count > 0 { _ = EnableWindow(HWND_BTN_OPEN, true); }
}

unsafe fn on_file_open_location() {
    let idx = send_msg(HWND_RESULT_LIST, LB_GETCURSEL, 0, 0).0 as usize;
    let st = state();
    if idx >= st.file_finder.results.len() { set_text(HWND_STATUS, "결과 목록에서 파일을 선택하세요."); return; }
    let path = st.file_finder.results[idx].clone();
    drop(st);
    open_in_explorer(&path);
    set_text(HWND_STATUS, &format!("탐색기에서 열기: {}", path.display()));
}

// ── 중복 검색 ──────────────────────────────────────────────────────────────

unsafe fn on_dup_scan(hwnd: HWND) {
    let folders = {
        let st = state();
        if st.folders.is_empty() { set_text(HWND_STATS, "폴더를 추가하세요."); return; }
        st.folders.clone()
    };

    {
        let mut st = state();
        *st.dup_finder.cancel.lock().unwrap() = false;
        st.dup_finder.groups.clear();
        st.dup_finder.result_labels.clear();
        st.dup_finder.result_paths.clear();
        st.dup_finder.selected.clear();
    }

    refresh_result_list();
    _ = EnableWindow(HWND_BTN_SCAN_DUP, false);
    _ = EnableWindow(HWND_BTN_ADD,      false);
    _ = EnableWindow(HWND_BTN_REMOVE,   false);
    _ = EnableWindow(HWND_BTN_CANCEL,   true);
    _ = EnableWindow(HWND_BTN_DELETE,   false);
    set_text(HWND_STATS, "스캔 중...");

    let cancel   = state().dup_finder.cancel.clone();
    let hwnd_raw = hwnd.0 as usize;

    std::thread::spawn(move || {
        let result = run_dup_scan(folders, cancel);
        unsafe {
            {
                let mut st = state();
                st.dup_finder.groups = result.groups;
            }
            LAST_TOTAL     = result.total_files;
            LAST_ELAPSED   = result.elapsed_secs;
            LAST_CANCELLED = result.cancelled;
            PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_SCAN_DONE_DUP, WPARAM(0), LPARAM(0)).ok();
        }
    });
}

unsafe fn on_dup_scan_done(_hwnd: HWND) {
    let groups_len = state().dup_finder.groups.len();
    let total   = LAST_TOTAL;
    let elapsed = LAST_ELAPSED;
    let cancelled = LAST_CANCELLED;

    let stats_base = format!("파일 {} · {}", total, format_duration(elapsed));

    if cancelled {
        set_text(HWND_STATS, &format!("[취소됨] 중복 그룹: {} | {}", groups_len, stats_base));
    } else if groups_len == 0 {
        set_text(HWND_STATS, &format!("중복 파일 없음. | {}", stats_base));
    } else {
        let waste: u64 = state().dup_finder.groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
        set_text(HWND_STATS, &format!("중복 그룹: {} | 낭비 공간: {} | {}", groups_len, format_size(waste), stats_base));
    }
    set_text(HWND_STATUS, "");

    { let mut st = state(); st.dup_finder.rebuild_results(); }
    refresh_result_list();

    if groups_len == 0 {
        let s = wstr("중복 파일이 없습니다.");
        send_msg(HWND_RESULT_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
    }

    _ = EnableWindow(HWND_BTN_SCAN_DUP, true);
    _ = EnableWindow(HWND_BTN_ADD,      true);
    _ = EnableWindow(HWND_BTN_REMOVE,   true);
    _ = EnableWindow(HWND_BTN_CANCEL,   false);
    if groups_len > 0 { _ = EnableWindow(HWND_BTN_DELETE, true); }
}

unsafe fn on_dup_result_click() {
    let idx = send_msg(HWND_RESULT_LIST, LB_GETCURSEL, 0, 0).0 as usize;
    let path = { let st = state(); if idx >= st.dup_finder.result_paths.len() { return; } st.dup_finder.result_paths[idx].clone() };
    let Some(path) = path else { return };

    {
        let mut st = state();
        if st.dup_finder.selected.contains(&path) { st.dup_finder.selected.remove(&path); }
        else { st.dup_finder.selected.insert(path.clone()); }
        let cnt = st.dup_finder.selected.len();
        st.dup_finder.rebuild_results();
        drop(st);
        if cnt > 0 { set_text(HWND_STATUS, &format!("{} 개 선택됨", cnt)); }
        else { set_text(HWND_STATUS, ""); }
    }

    refresh_result_list();
    send_msg(HWND_RESULT_LIST, LB_SETCURSEL, idx, 0);
}

unsafe fn on_dup_delete(hwnd: HWND) {
    let paths: Vec<PathBuf> = state().dup_finder.selected.iter().cloned().collect();
    if paths.is_empty() { return; }

    let msg   = wstr(&format!("{}개 파일을 삭제하시겠습니까? 되돌릴 수 없습니다.", paths.len()));
    let title = wstr("삭제 확인");
    let r = MessageBoxW(Some(hwnd), PCWSTR(msg.as_ptr()), PCWSTR(title.as_ptr()), MB_YESNO | MB_ICONQUESTION);
    if r != IDYES { return; }

    let mut failed = 0usize;
    for p in &paths { if std::fs::remove_file(p).is_err() { failed += 1; } }

    {
        let mut st = state();
        let selected = st.dup_finder.selected.clone();
        for g in &mut st.dup_finder.groups { g.files.retain(|f| !selected.contains(&f.path)); }
        st.dup_finder.groups.retain(|g| g.files.len() >= 2);
        st.dup_finder.selected.clear();
        st.dup_finder.rebuild_results();
    }

    refresh_result_list();
    let msg = if failed > 0 { format!("삭제 완료. ({}개 실패)", failed) } else { "삭제 완료.".to_string() };
    set_text(HWND_STATUS, &msg);

    let st = state();
    if st.dup_finder.groups.is_empty() {
        set_text(HWND_STATS, "중복 파일 없음.");
        _ = EnableWindow(HWND_BTN_DELETE, false);
    } else {
        let waste: u64 = st.dup_finder.groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
        set_text(HWND_STATS, &format!("중복 그룹: {} | 낭비 공간: {}", st.dup_finder.groups.len(), format_size(waste)));
    }
}

// ── 영상 체크 ──────────────────────────────────────────────────────────────

unsafe fn on_vid_scan(hwnd: HWND) {
    let folders = {
        let st = state();
        if st.folders.is_empty() { set_text(HWND_STATS, "폴더를 추가하세요."); return; }
        st.folders.clone()
    };

    let sample_frames = get_edit_text(HWND_EDIT_FRAMES).trim().parse::<usize>().unwrap_or(500).max(50).min(5000);
    let min_size_mb   = get_edit_text(HWND_EDIT_SIZE).trim().parse::<u64>().unwrap_or(1);
    let min_size_bytes = min_size_mb * 1024 * 1024;

    {
        let mut st = state();
        *st.video_checker.cancel.lock().unwrap() = false;
        st.video_checker.results.clear();
        st.video_checker.result_labels.clear();
        st.video_checker.result_paths.clear();
        st.video_checker.sample_frames = sample_frames;
        st.video_checker.min_file_size_mb = min_size_mb;
    }

    refresh_result_list();
    set_text(HWND_DETAIL, "");
    _ = EnableWindow(HWND_BTN_SCAN_VID, false);
    _ = EnableWindow(HWND_BTN_ADD,      false);
    _ = EnableWindow(HWND_BTN_REMOVE,   false);
    _ = EnableWindow(HWND_BTN_CANCEL,   true);
    _ = EnableWindow(HWND_BTN_OPEN_VID, false);
    set_text(HWND_STATUS, "파일 목록 수집 중...");
    set_text(HWND_STATS, "스캔 준비 중...");

    let cancel   = state().video_checker.cancel.clone();
    let hwnd_raw = hwnd.0 as usize;

    std::thread::spawn(move || {
        let result = run_vid_scan(folders, cancel, sample_frames, min_size_bytes, hwnd_raw);
        unsafe {
            { let mut st = state(); st.video_checker.results = result.results; }
            LAST_TOTAL     = result.total;
            LAST_ELAPSED   = result.elapsed_secs;
            LAST_CANCELLED = result.cancelled;
            PostMessageW(Some(HWND(hwnd_raw as *mut _)), WM_SCAN_DONE_VID, WPARAM(0), LPARAM(0)).ok();
        }
    });
}

unsafe fn on_vid_scan_done(_hwnd: HWND) {
    let total     = LAST_TOTAL;
    let elapsed   = LAST_ELAPSED;
    let cancelled = LAST_CANCELLED;

    { let mut st = state(); st.video_checker.rebuild_results(); }
    refresh_result_list();

    let (problems, warnings, normals) = {
        let st = state();
        let p = st.video_checker.results.iter().filter(|r| r.category == ResultCategory::Problem).count();
        let w = st.video_checker.results.iter().filter(|r| r.category == ResultCategory::Warning).count();
        let n = st.video_checker.results.iter().filter(|r| r.category == ResultCategory::Normal).count();
        (p, w, n)
    };

    if state().video_checker.result_labels.is_empty() {
        let s = wstr("영상 파일이 없습니다.");
        send_msg(HWND_RESULT_LIST, LB_ADDSTRING, 0, s.as_ptr() as isize);
    }

    let stats_str = if cancelled {
        format!("[취소됨] {}개 파일 스캔 | {}", total, format_duration(elapsed))
    } else {
        format!("완료. 문제: {} | 경고: {} | 정상: {}  (총 {}개, {})", problems, warnings, normals, total, format_duration(elapsed))
    };
    set_text(HWND_STATS, &stats_str);
    set_text(HWND_STATUS, "");

    _ = EnableWindow(HWND_BTN_SCAN_VID, true);
    _ = EnableWindow(HWND_BTN_ADD,      true);
    _ = EnableWindow(HWND_BTN_REMOVE,   true);
    _ = EnableWindow(HWND_BTN_CANCEL,   false);
}

unsafe fn on_vid_result_click() {
    let idx = send_msg(HWND_RESULT_LIST, LB_GETCURSEL, 0, 0).0 as usize;
    let path = {
        let st = state();
        if idx >= st.video_checker.result_paths.len() { return; }
        st.video_checker.result_paths[idx].clone()
    };

    let Some(path) = path else {
        set_text(HWND_DETAIL, "");
        _ = EnableWindow(HWND_BTN_OPEN_VID, false);
        return;
    };

    let detail = {
        let st = state();
        if let Some(r) = st.video_checker.results.iter().find(|r| r.video.path == path) {
            let br = if r.video.video_bit_rate > 0 { r.video.video_bit_rate } else { r.video.format_bit_rate };
            let mut lines = vec![
                format!("파일: {}", path.display()),
                format!("해상도: {}x{}  코덱: {}  FPS: {:.2}  비트레이트: {}/s  {}",
                    r.video.width, r.video.height, r.video.codec_name, r.video.avg_frame_rate,
                    format_size(br / 8),
                    if r.video.has_audio { "오디오: 있음" } else { "오디오: 없음" }),
            ];
            if r.issues.is_empty() {
                lines.push("  ✔ 이상 없음".to_string());
            } else {
                for issue in &r.issues {
                    lines.push(format!("  [{}] {}", issue.code(), issue.description()));
                }
            }
            lines.join("\r\n")
        } else { String::new() }
    };

    set_text(HWND_DETAIL, &detail);
    _ = EnableWindow(HWND_BTN_OPEN_VID, true);
}

unsafe fn on_vid_open_location() {
    let idx = send_msg(HWND_RESULT_LIST, LB_GETCURSEL, 0, 0).0 as usize;
    let st = state();
    if idx >= st.video_checker.result_paths.len() { return; }
    let Some(path) = st.video_checker.result_paths[idx].clone() else { return };
    drop(st);
    open_in_explorer(&path);
}

// ─────────────────────────────────────────────────────────────────────────────
// EDIT 서브클래스 (엔터키 → 검색)
// ─────────────────────────────────────────────────────────────────────────────

extern "system" fn edit_query_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_KEYDOWN && wparam.0 == VK_RETURN.0 as usize {
            on_file_search(HWND_MAIN);
            return LRESULT(0);
        }
        if let Some(orig) = EDIT_QUERY_ORIG_PROC {
            orig(hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WndProc
// ─────────────────────────────────────────────────────────────────────────────

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                HWND_MAIN = hwnd;
                let hinstance: HINSTANCE = GetModuleHandleW(PCWSTR::null()).unwrap().into();

                // 아이콘
                if let Ok(h) = LoadImageW(Some(hinstance), PCWSTR(wstr("icon.ico").as_ptr()), IMAGE_ICON, 0, 0, LR_LOADFROMFILE | LR_DEFAULTSIZE) {
                    send_msg(hwnd, WM_SETICON, ICON_BIG as usize, h.0 as isize);
                }
                if let Ok(h) = LoadImageW(Some(hinstance), PCWSTR(wstr("icon.ico").as_ptr()), IMAGE_ICON, 16, 16, LR_LOADFROMFILE) {
                    send_msg(hwnd, WM_SETICON, ICON_SMALL as usize, h.0 as isize);
                }

                // 폰트
                H_FONT = CreateFontW(20, 0, 0, 0, 400, 0, 0, 0,
                    FONT_CHARSET(0), FONT_OUTPUT_PRECISION(0), FONT_CLIP_PRECISION(0),
                    FONT_QUALITY(0), 0, PCWSTR(wstr("Segoe UI").as_ptr()));
                H_FONT_LARGE = CreateFontW(24, 0, 0, 0, 400, 0, 0, 0,
                    FONT_CHARSET(0), FONT_OUTPUT_PRECISION(0), FONT_CLIP_PRECISION(0),
                    FONT_QUALITY(0), 0, PCWSTR(wstr("Segoe UI").as_ptr()));
                H_FONT_TAB = CreateFontW(22, 0, 0, 0, 600, 0, 0, 0,
                    FONT_CHARSET(0), FONT_OUTPUT_PRECISION(0), FONT_CLIP_PRECISION(0),
                    FONT_QUALITY(0), 0, PCWSTR(wstr("Segoe UI").as_ptr()));

                // 브러시
                BRUSH_BG      = CreateSolidBrush(rgb(18, 18, 26));
                BRUSH_SURFACE = CreateSolidBrush(rgb(28, 28, 40));
                BRUSH_BTN     = CreateSolidBrush(rgb(40, 42, 58));
                BRUSH_EDIT    = CreateSolidBrush(rgb(35, 35, 50));
                BRUSH_TAB_ACT = CreateSolidBrush(rgb(60, 80, 140));

                let list_style = WS_CHILD | WS_VISIBLE | WS_BORDER | WS_VSCROLL |
                    WINDOW_STYLE((LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32);

                // ── 탭 버튼 ─────────────────────────────
                HWND_TAB_FILE  = create_control_font("BUTTON", "파일 검색",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_TAB_FILE, H_FONT_TAB);
                HWND_TAB_DUP   = create_control_font("BUTTON", "중복 파일 검출",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_TAB_DUP, H_FONT_TAB);
                HWND_TAB_VIDEO = create_control_font("BUTTON", "영상 품질 체크",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_TAB_VIDEO, H_FONT_TAB);

                // ── 공유 컨트롤 ─────────────────────────
                HWND_FOLDER_LIST = create_control_font("LISTBOX", "", list_style,
                    WS_EX_CLIENTEDGE, 0,0,0,0, hwnd, ID_FOLDER_LIST, H_FONT_LARGE);
                HWND_BTN_ADD    = create_control("BUTTON", "폴더 추가",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_ADD);
                HWND_BTN_REMOVE = create_control("BUTTON", "제거",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_REMOVE);
                HWND_RESULT_LIST = create_control_font("LISTBOX", "", list_style,
                    WS_EX_CLIENTEDGE, 0,0,0,0, hwnd, ID_RESULT_LIST, H_FONT_LARGE);
                HWND_STATUS = create_control_font("STATIC", "",
                    WS_CHILD | WS_VISIBLE, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_STATUS, H_FONT_LARGE);
                HWND_STATS = create_control_font("STATIC", "폴더를 추가하고 검색어를 입력하세요.",
                    WS_CHILD | WS_VISIBLE, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_STATS, H_FONT_LARGE);
                HWND_BTN_CANCEL = create_control("BUTTON", "취소",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_CANCEL);
                HWND_LBL_FOLDER = create_label(hwnd, "스캔 폴더", H_FONT_LARGE);
                HWND_LBL_RESULT = create_label(hwnd, "검색 결과", H_FONT_LARGE);

                // ── 파일 검색 전용 ──────────────────────
                HWND_LBL_QUERY = create_label(hwnd, "파일명 검색  (예: report, .pdf, 2024)", H_FONT_LARGE);
                HWND_EDIT_QUERY = create_control_font("EDIT", "",
                    WS_CHILD | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    WS_EX_CLIENTEDGE, 0,0,0,0, hwnd, ID_EDIT_QUERY, H_FONT_LARGE);
                HWND_BTN_SEARCH = create_control("BUTTON", "검색",
                    WS_CHILD | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_SEARCH);
                HWND_BTN_OPEN = create_control("BUTTON", "위치 열기",
                    WS_CHILD | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_OPEN);

                // EDIT 서브클래싱: 엔터키 → 검색
                let orig = SetWindowLongPtrW(HWND_EDIT_QUERY, GWLP_WNDPROC, edit_query_proc as *const () as isize);
                EDIT_QUERY_ORIG_PROC = Some(std::mem::transmute(orig));

                // ── 중복 검색 전용 ──────────────────────
                HWND_BTN_SCAN_DUP = create_control("BUTTON", "스캔",
                    WS_CHILD | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_SCAN_DUP);
                HWND_BTN_DELETE = create_control("BUTTON", "선택 항목 삭제",
                    WS_CHILD | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_DELETE);

                // ── 영상 체크 전용 ──────────────────────
                HWND_BTN_SCAN_VID = create_control("BUTTON", "스캔",
                    WS_CHILD | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_SCAN_VID);
                HWND_EDIT_FRAMES = create_control_font("EDIT", "500",
                    WS_CHILD | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_NUMBER as u32 | ES_AUTOHSCROLL as u32),
                    WS_EX_CLIENTEDGE, 0,0,0,0, hwnd, ID_EDIT_FRAMES, H_FONT);
                HWND_EDIT_SIZE = create_control_font("EDIT", "1",
                    WS_CHILD | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_NUMBER as u32 | ES_AUTOHSCROLL as u32),
                    WS_EX_CLIENTEDGE, 0,0,0,0, hwnd, ID_EDIT_SIZE, H_FONT);
                HWND_DETAIL = create_control_font("STATIC", "",
                    WS_CHILD,
                    WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_DETAIL_TEXT, H_FONT);
                HWND_BTN_OPEN_VID = create_control("BUTTON", "위치 열기",
                    WS_CHILD | WS_TABSTOP, WINDOW_EX_STYLE(0), 0,0,0,0, hwnd, ID_BTN_OPEN_VID);
                HWND_LBL_FRAMES = create_label(hwnd, "샘플 프레임:", H_FONT);
                HWND_LBL_SIZE   = create_label(hwnd, "최소 크기(MB):", H_FONT);

                // 초기 비활성화
                _ = EnableWindow(HWND_BTN_CANCEL,   false);
                _ = EnableWindow(HWND_BTN_OPEN,     false);
                _ = EnableWindow(HWND_BTN_DELETE,   false);
                _ = EnableWindow(HWND_BTN_OPEN_VID, false);

                // 탭 전환: 기본 파일 검색
                show_tab(Tab::FileFinder);
                layout(hwnd);
                LRESULT(0)
            }

            WM_SIZE => { layout(hwnd); LRESULT(0) }

            WM_COMMAND => {
                let id    = (wparam.0 & 0xFFFF) as u16;
                let notif = ((wparam.0 >> 16) & 0xFFFF) as u16;
                let tab   = { state().current_tab };

                match id {
                    // ── 탭 버튼 ──────────────────────────
                    ID_TAB_FILE  => show_tab(Tab::FileFinder),
                    ID_TAB_DUP   => show_tab(Tab::DuplicateFinder),
                    ID_TAB_VIDEO => show_tab(Tab::VideoChecker),

                    // ── 공유 ─────────────────────────────
                    ID_BTN_ADD    => on_add_folder(hwnd),
                    ID_BTN_REMOVE => on_remove_folder(),
                    ID_BTN_CANCEL => {
                        let cancel_arc = {
                            let st = state();
                            match tab {
                                Tab::FileFinder      => st.file_finder.cancel.clone(),
                                Tab::DuplicateFinder => st.dup_finder.cancel.clone(),
                                Tab::VideoChecker    => st.video_checker.cancel.clone(),
                            }
                        };
                        *cancel_arc.lock().unwrap() = true;
                    }

                    // ── 파일 검색 ────────────────────────
                    ID_BTN_SEARCH => on_file_search(hwnd),
                    ID_BTN_OPEN   => on_file_open_location(),

                    // ── 중복 검색 ────────────────────────
                    ID_BTN_SCAN_DUP => on_dup_scan(hwnd),
                    ID_BTN_DELETE   => on_dup_delete(hwnd),
                    ID_RESULT_LIST if notif == LBN_SELCHANGE as u16 => {
                        match tab {
                            Tab::DuplicateFinder => on_dup_result_click(),
                            Tab::VideoChecker   => on_vid_result_click(),
                            _ => {}
                        }
                    }

                    // ── 영상 체크 ────────────────────────
                    ID_BTN_SCAN_VID  => on_vid_scan(hwnd),
                    ID_BTN_OPEN_VID  => on_vid_open_location(),

                    _ => {}
                }
                LRESULT(0)
            }

            WM_KEYDOWN => {
                if wparam.0 == VK_RETURN.0 as usize && state().current_tab == Tab::FileFinder {
                    on_file_search(hwnd);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            WM_SEARCH_DONE   => { on_file_search_done(hwnd); LRESULT(0) }
            WM_SCAN_DONE_DUP => { on_dup_scan_done(hwnd);    LRESULT(0) }
            WM_SCAN_PROGRESS => {
                let done  = wparam.0;
                let total = lparam.0 as usize;
                set_text(HWND_STATUS, &format!("스캔 중... {}/{} 파일", done, total));
                LRESULT(0)
            }
            WM_SCAN_DONE_VID => { on_vid_scan_done(hwnd); LRESULT(0) }

            WM_CTLCOLORSTATIC => {
                let hdc = HDC(wparam.0 as *mut _);
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, rgb(230, 230, 240));
                LRESULT(BRUSH_BG.0 as isize)
            }
            WM_CTLCOLORLISTBOX => {
                let hdc = HDC(wparam.0 as *mut _);
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, rgb(230, 230, 240));
                SetBkColor(hdc, rgb(28, 28, 40));
                LRESULT(BRUSH_SURFACE.0 as isize)
            }
            WM_CTLCOLOREDIT => {
                let hdc = HDC(wparam.0 as *mut _);
                SetBkMode(hdc, OPAQUE);
                SetTextColor(hdc, rgb(230, 230, 240));
                SetBkColor(hdc, rgb(35, 35, 50));
                LRESULT(BRUSH_EDIT.0 as isize)
            }
            WM_CTLCOLORBTN => {
                let hdc = HDC(wparam.0 as *mut _);
                let hwnd_ctrl = HWND(lparam.0 as *mut _);
                // 활성 탭 버튼은 다른 색
                let tab = { state().current_tab };
                let is_active_tab =
                    (tab == Tab::FileFinder     && hwnd_ctrl == HWND_TAB_FILE) ||
                    (tab == Tab::DuplicateFinder && hwnd_ctrl == HWND_TAB_DUP) ||
                    (tab == Tab::VideoChecker   && hwnd_ctrl == HWND_TAB_VIDEO);
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, if is_active_tab { rgb(255, 255, 255) } else { rgb(200, 200, 215) });
                LRESULT(if is_active_tab { BRUSH_TAB_ACT.0 as isize } else { BRUSH_BTN.0 as isize })
            }
            WM_ERASEBKGND => {
                let hdc = HDC(wparam.0 as *mut _);
                let mut rc = RECT::default();
                GetClientRect(hwnd, &mut rc).ok();
                FillRect(hdc, &rc, BRUSH_BG);
                LRESULT(1)
            }
            WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// main
// ─────────────────────────────────────────────────────────────────────────────

fn main() {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok();

        // ffprobe 가용성 미리 확인 (비차단)
        let ffprobe_ok = std::process::Command::new("ffprobe")
            .arg("-version").creation_flags(CREATE_NO_WINDOW).output()
            .map(|o| o.status.success()).unwrap_or(false);

        APP_STATE.set(Mutex::new(AppState::new())).ok();

        let hinstance: HINSTANCE = GetModuleHandleW(PCWSTR::null()).unwrap().into();
        let class_name = wstr("MediaInspector");

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            ..Default::default()
        };
        RegisterClassExW(&wc);

        let title = wstr("Media Inspector");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT, CW_USEDEFAULT, 1150, 820,
            None, None, Some(hinstance), None,
        ).unwrap();

        if !ffprobe_ok {
            let msg = wstr("ffprobe를 찾을 수 없습니다.\n\n영상 품질 체크 기능을 사용하려면 ffprobe.exe를 PATH에 추가하거나\n실행 파일과 같은 폴더에 복사하세요.\n\nFFmpeg 다운로드: https://ffmpeg.org/download.html");
            let title = wstr("ffprobe 없음 — Media Inspector");
            MessageBoxW(Some(hwnd), PCWSTR(msg.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONWARNING);
        }

        _ = ShowWindow(hwnd, SW_SHOW);
        _ = UpdateWindow(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
