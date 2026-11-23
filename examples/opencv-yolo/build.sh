#!/usr/bin/env bash
# Build everything this sample needs, into lib/, third_party/ and models/.
# Steps whose output already exists are skipped.
#
# Needs: a C/C++ compiler, cmake, python3, the examples' virtualenv ../.venv
# (or set PYTHON=...) with pip or uv for the model export, and FFmpeg development
# packages for OpenCV's video input (Debian/Ubuntu: sudo apt install cmake
# libavcodec-dev libavformat-dev libswscale-dev).
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p lib models third_party
PYTHON="${PYTHON:-../.venv/bin/python}"

# 1. raylib 5.5 (the window).
bash ../fetch-raylib.sh lib

# 2. ONNX Runtime 1.32.0: the shared library shipped in its pip wheel. The C API
#    headers in include/ are from the same release.
if [ ! -e lib/libonnxruntime.so.1.32.0 ]; then
    python3 - <<'PY'
import io, json, urllib.request, zipfile
meta = json.load(urllib.request.urlopen("https://pypi.org/pypi/onnxruntime/1.32.0/json"))
url = next(f["url"] for f in meta["urls"] if "manylinux" in f["filename"] and f["filename"].endswith("x86_64.whl"))
wheel = zipfile.ZipFile(io.BytesIO(urllib.request.urlopen(url).read()))
open("lib/libonnxruntime.so.1.32.0", "wb").write(wheel.read("onnxruntime/capi/libonnxruntime.so.1.32.0"))
PY
fi

# 3. OpenCV 4.10.0, only the modules used (camera and video input, image ops).
if [ ! -d third_party/opencv4/lib ]; then
    tmp="$(mktemp -d)"
    curl -fsSL https://github.com/opencv/opencv/archive/refs/tags/4.10.0.tar.gz | tar -xz -C "$tmp"
    cmake -S "$tmp/opencv-4.10.0" -B "$tmp/build" -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$PWD/third_party/opencv4" \
        -DBUILD_LIST=core,imgproc,imgcodecs,videoio -DWITH_FFMPEG=ON \
        -DBUILD_TESTS=OFF -DBUILD_PERF_TESTS=OFF -DBUILD_EXAMPLES=OFF -DBUILD_opencv_apps=OFF
    cmake --build "$tmp/build" -j "$(nproc)"
    cmake --install "$tmp/build"
    rm -rf "$tmp"
fi

# 4. The YOLO26 nano models (detection, segmentation, pose), exported to ONNX
#    with Ultralytics. Note the Ultralytics models' licence (AGPL-3.0).
if [ ! -e models/yolo26n-pose.onnx ]; then
    if "$PYTHON" -m pip --version >/dev/null 2>&1; then
        "$PYTHON" -m pip install ultralytics
    else
        uv pip install --python "$PYTHON" ultralytics
    fi
    (cd models && "$PYTHON" -c "
from ultralytics import YOLO
for m in ('yolo26n.pt', 'yolo26n-seg.pt', 'yolo26n-pose.pt'):
    YOLO(m).export(format='onnx', imgsz=640)
")
fi

# 5. lib/libvision.so: the C++ part (camera, inference, drawing).
g++ -std=c++17 -O2 -shared -fPIC src/vision.cpp -o lib/libvision.so \
    -Iinclude -I../raylib -Ithird_party/opencv4/include/opencv4 \
    -Llib -Lthird_party/opencv4/lib \
    -lopencv_videoio -lopencv_imgproc -lopencv_core -l:libraylib.so.550 -ldl \
    -Wl,-rpath,'$ORIGIN:$ORIGIN/../third_party/opencv4/lib'
echo "built lib/libvision.so"
