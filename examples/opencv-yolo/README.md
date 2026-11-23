# opencv-yolo

Live detection, segmentation, and pose. `vision.c` is the module pyrock compiles. It loads `lib/libvision.so`, which is C++ linked against three C libraries:

- OpenCV (`third_party/opencv4`) opens the camera or a video file and prepares each frame
- ONNX Runtime (`lib/libonnxruntime.so.1.32.0`) runs the official YOLO26 nano models
- raylib (`lib/libraylib.so.550`) opens the window

The models are `models/yolo26n.onnx`, `models/yolo26n-seg.onnx`, and `models/yolo26n-pose.onnx`. The window shows detection, segmentation, pose, and a 3D character that follows the detected pose. The camera orbits that character. With no extra argument it opens the first working camera. A number selects a camera. Anything else is a video file, and a video restarts at the end. Press `q` or Esc to quit.

<!-- readme_check: skip-ci (needs a camera or video, OpenCV and the YOLO models) -->

`bash build.sh` prepares everything once (see the script for what it needs): raylib, ONNX
Runtime from its PyPI wheel, an OpenCV 4.10 build with only the modules used, the YOLO26
models exported to ONNX with Ultralytics (their licence is AGPL-3.0), and `lib/libvision.so`.
The last command plays a video file; `clip.mp4` stands for any video you have.

```sh
bash build.sh
pyrock check main.py
pyrock explain main.py
pyrock run main.py
pyrock run main.py -- 1
pyrock run main.py -- clip.mp4
```
