# Media Inspector

Windows용 미디어 파일 관리 도구. 파일 검색, 중복 파일 검출, 영상 품질 체크 기능을 하나의 창에서 제공합니다.

## 기능

### 파일 검색
- 여러 폴더를 등록해 파일명 키워드로 재귀 검색
- 검색어 입력 후 **Enter** 또는 **검색** 버튼으로 실행
- 결과에서 파일 선택 → **위치 열기**로 탐색기에서 바로 확인

### 중복 파일 검출
- 등록된 폴더를 재귀 스캔해 동일 파일(부분 MD5 해시) 그룹화
- 중복 파일 선택 후 일괄 삭제 가능
- 낭비 공간 합산 표시

### 영상 품질 체크
- ffprobe 기반 영상 분석 (VFR, 프레임 드롭, 코덱 호환성, A/V 싱크 등 15가지 항목)
- 문제(Problem) / 경고(Warning) / 정상(OK) 3단계 분류
- 샘플 프레임 수, 최소 파일 크기(MB) 조절 가능
- 결과 선택 시 상세 정보(해상도, 코덱, FPS, 비트레이트, 이슈 목록) 표시

### 공통
- 세 탭이 폴더 목록을 공유 — 한 번 폴더 추가하면 모든 기능에서 사용
- 작업 중 **취소** 버튼으로 즉시 중단 가능
- 다크 테마 UI (Segoe UI)

## 요구 사항

- Windows 10 이상 (x86-64)
- **영상 품질 체크**: [FFmpeg](https://ffmpeg.org/download.html)의 `ffprobe.exe`가 PATH 또는 실행 파일과 같은 폴더에 있어야 함

## 빌드

```powershell
# Rust 설치: https://rustup.rs
.\build.ps1
# 결과물: dist\MediaInspector.exe
```

또는 직접:

```powershell
cargo build --release
# 결과물: target\release\MediaInspector.exe
```

## 이슈 코드

| 코드 | 설명 |
|------|------|
| VFR | 가변 프레임레이트 |
| DROP | 프레임 드롭 |
| CORRUPT | 손상/누락 프레임 |
| COMPAT | 코덱/프로파일 호환성 위험 |
| AVSYNC | 오디오·영상 싱크 오류 |
| BSPK | 비트레이트 급등 |
| GOP | 키프레임 간격 과다 (>10s) |
| LOWBR | 해상도 대비 낮은 비트레이트 |
| CTRMM | 컨테이너·코덱 불일치 |
| NOAUD | 오디오 스트림 없음 |
| GOPI | 불규칙한 키프레임 간격 |
| DUR | 비정상 재생 시간 |
| HIBR | 비정상적으로 높은 비트레이트 |
| RES | 비표준 해상도 |
| ROT | 회전 메타데이터 (세로 영상) |
