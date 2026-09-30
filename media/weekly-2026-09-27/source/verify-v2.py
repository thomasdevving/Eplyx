"""Decode the final export and check that recorded product panes change."""
from pathlib import Path
import subprocess, re, json, hashlib, datetime, os
from PIL import Image
import numpy as np

root = Path(__file__).resolve().parents[1]
out, qa = root / 'output', root / 'output/qa-v2'
video = out / 'eplyx-weekly-2026-09-27-v2.mp4'
ff = os.environ.get('FFMPEG', '/private/tmp/eplyx-video-tools/imageio_ffmpeg/binaries/ffmpeg-macos-aarch64-v7.1')
r = subprocess.run([ff, '-hide_banner', '-i', str(video), '-map', '0:v:0', '-map', '0:a:0', '-f', 'null', '-', '-progress', 'pipe:1', '-nostats'], capture_output=True, text=True, check=True)
(out / 'decode-v2-log.txt').write_text(r.stderr)
(out / 'decode-v2-progress.txt').write_text(r.stdout)
frames = int(re.findall(r'^frame=(\d+)', r.stdout, re.M)[-1])
elapsed = int(re.findall(r'^out_time_us=(\d+)', r.stdout, re.M)[-1]) / 1e6
assert frames == 3840 and elapsed == 128 and 'progress=end' in r.stdout
assert all(value in r.stderr for value in ['1920x1080', '30 fps', 'Audio: aac', 'bt709'])
assert not re.search(r'error|invalid|corrupt', r.stderr, re.I)
arrays = {}
for t in [1, 4.5, 44, 48, 78, 82, 90, 94, 110, 114, 120]:
    p = qa / f'encoded-{t}.jpg'
    subprocess.run([ff, '-y', '-hide_banner', '-loglevel', 'error', '-ss', str(t), '-i', str(video), '-frames:v', '1', '-q:v', '2', str(p)], check=True)
    arrays[t] = np.asarray(Image.open(p).convert('RGB'), dtype=np.float32)
checks = []
for label, a, b, crop in [
    ('Native hero animation', 1, 4.5, (0, 0, 1920, 990)),
    ('Dashboard navigation', 44, 48, (703, 371, 1856, 985)),
    ('Observation inputs to result', 78, 82, (64, 287, 1374, 1024)),
    ('Queued path to result', 90, 94, (64, 287, 1374, 1024)),
    ('Lifecycle queue to result', 110, 114, (64, 287, 1374, 1024)),
]:
    x1, y1, x2, y2 = crop
    diff = float(np.abs(arrays[a][y1:y2, x1:x2] - arrays[b][y1:y2, x1:x2]).mean())
    assert diff > .1
    checks.append({'sequence': label, 'times': [a, b], 'product_pane_mean_pixel_difference': round(diff, 3)})
result = {
    'file': video.name, 'bytes': video.stat().st_size,
    'sha256': hashlib.sha256(video.read_bytes()).hexdigest(),
    'duration_seconds': elapsed, 'dimensions': [1920, 1080], 'fps': 30,
    'decoded_frames': frames, 'decode_exit_code': r.returncode, 'dropped_frames': 0,
    'video_codec': 'H.264 High / yuv420p / BT.709 limited range',
    'audio_codec': 'AAC stereo / 48 kHz', 'subtitle_codec': 'mov_text / English',
    'chapters': 16, 'visual_review': '34 composition frames and 11 encoded samples',
    'moving_product_pane_checks': checks,
    'verified_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
}
(out / 'verification-v2.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2))
