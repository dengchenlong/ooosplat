"""Verify real-SAM2 smoke masks track the drifting red square (CPU run)."""
import sys
from pathlib import Path
from PIL import Image

masks_dir = Path(sys.argv[1])
frames = [f"{i:04d}" for i in range(4)]
ok = True
for i, fid in enumerate(frames):
    img = Image.open(masks_dir / f"{fid}.png").convert("L")
    W, H = img.size
    px = img.load()
    # expected square: x in [20+3i, 45+3i), y in [15, 45) -> 25x30=750 px
    exp_center = (32 + 3 * i, 30)
    inside = px[exp_center]
    outside = px[10, 60]  # background corner
    print(f"{fid}: center={inside} bg={outside} size={W}x{H}")
    if inside != 255 or outside != 0:
        ok = False
print("MASK CONTENT VERIFY:", "OK" if ok else "MISMATCH")
sys.exit(0 if ok else 1)
