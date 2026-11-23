// Camera and video come from the OpenCV C++ library. The three YOLO26 graphs
// run through the ONNX Runtime C API. The window is raylib.
#include <onnxruntime_c_api.h>

#include <dlfcn.h>

#include "raylib.h"

#include <opencv2/imgproc.hpp>
#include <opencv2/videoio.hpp>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>

// The sample folder: libvision.so lives in its lib/ folder (found with dladdr).
static std::string root() {
    Dl_info info;
    std::string me;
    if (dladdr((void *)&root, &info) && info.dli_fname) {
        me = info.dli_fname;
    }
    std::string dir = me.substr(0, me.find_last_of('/'));
    return dir.substr(0, dir.find_last_of('/'));
}
static const int SIZE = 640;
static const float CONF = 0.35f;
static const float KEYPOINT_CONF = 0.4f;
static const int BONES[][2] = {
    {0, 1}, {0, 2}, {1, 3}, {2, 4}, {5, 6}, {5, 7}, {7, 9}, {6, 8}, {8, 10},
    {5, 11}, {6, 12}, {11, 12}, {11, 13}, {13, 15}, {12, 14}, {14, 16},
};
static const char *COCO[] = {
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train", "truck", "boat",
    "traffic light", "fire hydrant", "stop sign", "parking meter", "bench", "bird", "cat", "dog", "horse", "sheep", "cow",
    "elephant", "bear", "zebra", "giraffe", "backpack", "umbrella", "handbag", "tie", "suitcase", "frisbee",
    "skis", "snowboard", "sports ball", "kite", "baseball bat", "baseball glove", "skateboard", "surfboard", "tennis racket", "bottle",
    "wine glass", "cup", "fork", "knife", "spoon", "bowl", "banana", "apple", "sandwich", "orange",
    "broccoli", "carrot", "hot dog", "pizza", "donut", "cake", "chair", "couch", "potted plant", "bed",
    "dining table", "toilet", "tv", "laptop", "mouse", "remote", "keyboard", "cell phone", "microwave", "oven",
    "toaster", "sink", "refrigerator", "book", "clock", "vase", "scissors", "teddy bear", "hair drier", "toothbrush",
};
static const int COCO_N = 80;
static char STATUS[512];

struct Tensor {
    std::vector<float> data;
    std::vector<int64_t> shape;
};

struct Net {
    OrtSession *session = nullptr;
    std::string input;
    std::vector<std::string> outputs;
};

struct Letter {
    std::vector<float> blob;
    float scale;
    int left;
    int top;
    int resized_w;
    int resized_h;
};

static void check(const OrtApi *api, OrtStatus *status) {
    if (!status) {
        return;
    }
    std::string message = api->GetErrorMessage(status);
    api->ReleaseStatus(status);
    throw std::runtime_error(message);
}

static std::string cmdline_source() {
    FILE *file = fopen("/proc/self/cmdline", "rb");
    if (!file) {
        return "";
    }
    std::string data;
    char block[256];
    size_t n = 0;
    while ((n = fread(block, 1, sizeof block, file)) > 0) {
        data.append(block, n);
    }
    fclose(file);
    int field = 0;
    std::string current;
    for (size_t i = 0; i < data.size(); i++) {
        if (data[i] != '\0') {
            current.push_back(data[i]);
            continue;
        }
        if (field == 1) {
            return current;
        }
        field += 1;
        current.clear();
    }
    return "";
}

static bool is_file(const std::string &path) {
    FILE *file = fopen(path.c_str(), "rb");
    if (!file) {
        return false;
    }
    fclose(file);
    return true;
}

static bool all_digits(const std::string &text) {
    if (text.empty()) {
        return false;
    }
    for (char c : text) {
        if (c < '0' || c > '9') {
            return false;
        }
    }
    return true;
}

static std::string base_name(const std::string &path) {
    size_t slash = path.find_last_of('/');
    if (slash == std::string::npos) {
        return path;
    }
    return path.substr(slash + 1);
}

static Net load_net(const OrtApi *api, OrtEnv *env, OrtAllocator *alloc, const std::string &path) {
    OrtSessionOptions *options = nullptr;
    check(api, api->CreateSessionOptions(&options));
    check(api, api->SetIntraOpNumThreads(options, 8));
    check(api, api->SetInterOpNumThreads(options, 1));
    check(api, api->SetSessionGraphOptimizationLevel(options, ORT_ENABLE_ALL));
    check(api, api->AddSessionConfigEntry(options, "session.intra_op.allow_spinning", "0"));
    check(api, api->AddSessionConfigEntry(options, "session.inter_op.allow_spinning", "0"));
    Net net;
    check(api, api->CreateSession(env, path.c_str(), options, &net.session));
    api->ReleaseSessionOptions(options);
    char *name = nullptr;
    check(api, api->SessionGetInputName(net.session, 0, alloc, &name));
    net.input = name;
    check(api, api->AllocatorFree(alloc, name));
    size_t count = 0;
    check(api, api->SessionGetOutputCount(net.session, &count));
    for (size_t i = 0; i < count; i++) {
        check(api, api->SessionGetOutputName(net.session, i, alloc, &name));
        net.outputs.emplace_back(name);
        check(api, api->AllocatorFree(alloc, name));
    }
    return net;
}

static std::vector<Tensor> run_net(const OrtApi *api, OrtMemoryInfo *memory, Net &net, std::vector<float> &blob) {
    int64_t shape[4] = {1, 3, SIZE, SIZE};
    OrtValue *input = nullptr;
    check(api, api->CreateTensorWithDataAsOrtValue(
                   memory, blob.data(), blob.size() * sizeof(float), shape, 4,
                   ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT, &input));
    const char *input_names[1] = {net.input.c_str()};
    std::vector<const char *> output_names;
    for (const std::string &name : net.outputs) {
        output_names.push_back(name.c_str());
    }
    std::vector<OrtValue *> outputs(output_names.size(), nullptr);
    check(api, api->Run(net.session, nullptr, input_names, &input, 1, output_names.data(), output_names.size(), outputs.data()));
    std::vector<Tensor> tensors;
    for (OrtValue *value : outputs) {
        Tensor tensor;
        OrtTensorTypeAndShapeInfo *info = nullptr;
        check(api, api->GetTensorTypeAndShape(value, &info));
        size_t dims = 0;
        check(api, api->GetDimensionsCount(info, &dims));
        tensor.shape.resize(dims);
        check(api, api->GetDimensions(info, tensor.shape.data(), dims));
        api->ReleaseTensorTypeAndShapeInfo(info);
        int64_t count = 1;
        for (int64_t dim : tensor.shape) {
            count *= dim;
        }
        float *data = nullptr;
        check(api, api->GetTensorMutableData(value, (void **)&data));
        tensor.data.assign(data, data + count);
        tensors.push_back(std::move(tensor));
        api->ReleaseValue(value);
    }
    api->ReleaseValue(input);
    return tensors;
}

static Letter letterbox(const cv::Mat &frame) {
    float scale = std::min(SIZE / (float)frame.rows, SIZE / (float)frame.cols);
    int resized_w = (int)std::lround(frame.cols * scale);
    int resized_h = (int)std::lround(frame.rows * scale);
    cv::Mat resized;
    cv::resize(frame, resized, cv::Size(resized_w, resized_h), 0, 0, cv::INTER_LINEAR);
    float pad_x = (SIZE - resized_w) / 2.0f;
    float pad_y = (SIZE - resized_h) / 2.0f;
    int top = (int)std::lround(pad_y - 0.1f);
    int bottom = (int)std::lround(pad_y + 0.1f);
    int left = (int)std::lround(pad_x - 0.1f);
    int right = (int)std::lround(pad_x + 0.1f);
    cv::Mat canvas;
    cv::copyMakeBorder(resized, canvas, top, bottom, left, right, cv::BORDER_CONSTANT, cv::Scalar(114, 114, 114));
    if (canvas.rows != SIZE || canvas.cols != SIZE) {
        cv::resize(canvas, canvas, cv::Size(SIZE, SIZE));
    }
    Letter out;
    out.scale = scale;
    out.left = left;
    out.top = top;
    out.resized_w = resized_w;
    out.resized_h = resized_h;
    out.blob.assign(3 * SIZE * SIZE, 0.0f);
    for (int y = 0; y < SIZE; y++) {
        const unsigned char *row = canvas.ptr<unsigned char>(y);
        for (int x = 0; x < SIZE; x++) {
            float b = row[x * 3] / 255.0f;
            float g = row[x * 3 + 1] / 255.0f;
            float r = row[x * 3 + 2] / 255.0f;
            int pixel = y * SIZE + x;
            out.blob[pixel] = r;
            out.blob[SIZE * SIZE + pixel] = g;
            out.blob[2 * SIZE * SIZE + pixel] = b;
        }
    }
    return out;
}

static float cell(const Tensor &tensor, int row, int col, int cols) {
    return tensor.data[(size_t)row * (size_t)cols + (size_t)col];
}

static void undo_box(float &x1, float &y1, float &x2, float &y2, const Letter &box, int width, int height) {
    if (std::max(x2, y2) <= 1.5f) {
        x1 *= SIZE;
        y1 *= SIZE;
        x2 *= SIZE;
        y2 *= SIZE;
    }
    x1 = (x1 - box.left) / box.scale;
    y1 = (y1 - box.top) / box.scale;
    x2 = (x2 - box.left) / box.scale;
    y2 = (y2 - box.top) / box.scale;
    x1 = std::min((float)width - 1.0f, std::max(0.0f, x1));
    y1 = std::min((float)height - 1.0f, std::max(0.0f, y1));
    x2 = std::min((float)width - 1.0f, std::max(0.0f, x2));
    y2 = std::min((float)height - 1.0f, std::max(0.0f, y2));
}

static cv::Scalar color_of(int index) {
    int b = 50 + (index * 47) % 180;
    int g = 50 + (index * 91) % 180;
    int r = 80 + (index * 29) % 160;
    return cv::Scalar(b, g, r);
}

static void draw_box(cv::Mat &image, float x1, float y1, float x2, float y2, const std::string &label, cv::Scalar color) {
    int xa = (int)x1;
    int ya = (int)y1;
    cv::rectangle(image, cv::Point(xa, ya), cv::Point((int)x2, (int)y2), color, 2);
    int baseline = 0;
    cv::Size size = cv::getTextSize(label, cv::FONT_HERSHEY_SIMPLEX, 0.5, 1, &baseline);
    int y0 = std::max(0, ya - size.height - 6);
    cv::rectangle(image, cv::Point(xa, y0), cv::Point(xa + size.width + 6, y0 + size.height + 6), color, cv::FILLED);
    cv::putText(image, label, cv::Point(xa + 3, y0 + size.height + 2), cv::FONT_HERSHEY_SIMPLEX, 0.5, cv::Scalar(24, 16, 20), 1, cv::LINE_AA);
}

static void paint_title(cv::Mat &image, const std::string &text) {
    cv::rectangle(image, cv::Point(0, 0), cv::Point(image.cols, 32), cv::Scalar(28, 14, 16), cv::FILLED);
    cv::putText(image, text, cv::Point(10, 22), cv::FONT_HERSHEY_SIMPLEX, 0.6, cv::Scalar(120, 220, 255), 1, cv::LINE_AA);
}

static const Tensor *grid_of(const std::vector<Tensor> &tensors, int last) {
    for (const Tensor &tensor : tensors) {
        if (tensor.shape.size() == 3 && tensor.shape[2] == last) {
            return &tensor;
        }
    }
    for (const Tensor &tensor : tensors) {
        if (tensor.shape.size() == 3) {
            return &tensor;
        }
    }
    return nullptr;
}

static const Tensor *proto_of(const std::vector<Tensor> &tensors) {
    for (const Tensor &tensor : tensors) {
        if (tensor.shape.size() == 4) {
            return &tensor;
        }
    }
    return nullptr;
}

static void draw_detections(cv::Mat &image, const Tensor &det, const Letter &box) {
    int rows = (int)det.shape[1];
    int cols = (int)det.shape[2];
    for (int row = 0; row < rows; row++) {
        float score = cell(det, row, 4, cols);
        if (score < CONF) {
            continue;
        }
        float x1 = cell(det, row, 0, cols);
        float y1 = cell(det, row, 1, cols);
        float x2 = cell(det, row, 2, cols);
        float y2 = cell(det, row, 3, cols);
        undo_box(x1, y1, x2, y2, box, image.cols, image.rows);
        if (x2 - x1 < 2.0f || y2 - y1 < 2.0f) {
            continue;
        }
        int index = (int)std::lround(cell(det, row, 5, cols));
        if (index < 0 || index >= COCO_N) {
            continue;
        }
        char label[96];
        snprintf(label, sizeof label, "%s %.2f", COCO[index], score);
        draw_box(image, x1, y1, x2, y2, label, color_of(index));
    }
}

static void draw_pose(cv::Mat &image, const Tensor &det, const Letter &box) {
    int rows = (int)det.shape[1];
    int cols = (int)det.shape[2];
    for (int row = 0; row < rows; row++) {
        float score = cell(det, row, 4, cols);
        if (score < CONF) {
            continue;
        }
        float x1 = cell(det, row, 0, cols);
        float y1 = cell(det, row, 1, cols);
        float x2 = cell(det, row, 2, cols);
        float y2 = cell(det, row, 3, cols);
        undo_box(x1, y1, x2, y2, box, image.cols, image.rows);
        char label[64];
        snprintf(label, sizeof label, "person %.2f", score);
        draw_box(image, x1, y1, x2, y2, label, cv::Scalar(255, 220, 80));
        std::vector<cv::Point2f> points;
        std::vector<float> confidence;
        for (int k = 6; k + 2 < cols; k += 3) {
            float px = cell(det, row, k, cols);
            float py = cell(det, row, k + 1, cols);
            float pc = cell(det, row, k + 2, cols);
            if (std::max(px, py) <= 1.5f) {
                px *= SIZE;
                py *= SIZE;
            }
            px = (px - box.left) / box.scale;
            py = (py - box.top) / box.scale;
            points.push_back(cv::Point2f(px, py));
            confidence.push_back(pc);
        }
        for (const int *bone : BONES) {
            int a = bone[0];
            int b = bone[1];
            if (a >= (int)points.size() || b >= (int)points.size()) {
                continue;
            }
            if (confidence[a] < KEYPOINT_CONF || confidence[b] < KEYPOINT_CONF) {
                continue;
            }
            cv::line(image, points[a], points[b], cv::Scalar(80, 180, 255), 2, cv::LINE_AA);
        }
        for (size_t i = 0; i < points.size(); i++) {
            if (confidence[i] < KEYPOINT_CONF) {
                continue;
            }
            cv::circle(image, points[i], 3, cv::Scalar(180, 255, 40), cv::FILLED, cv::LINE_AA);
        }
    }
}

static void draw_segments(cv::Mat &image, const Tensor &det, const Tensor &proto, const Letter &box) {
    int rows = (int)det.shape[1];
    int cols = (int)det.shape[2];
    if (cols < 38) {
        draw_detections(image, det, box);
        return;
    }
    std::vector<int> order;
    for (int row = 0; row < rows; row++) {
        if (cell(det, row, 4, cols) >= CONF) {
            order.push_back(row);
        }
    }
    std::sort(order.begin(), order.end(), [&](int a, int b) {
        return cell(det, a, 4, cols) > cell(det, b, 4, cols);
    });
    if (order.size() > 8) {
        order.resize(8);
    }
    int proto_pixels = 160 * 160;
    for (size_t n = 0; n < order.size(); n++) {
        int row = order[n];
        float coeff[32];
        for (int c = 0; c < 32; c++) {
            coeff[c] = cell(det, row, 6 + c, cols);
        }
        cv::Mat mask160(160, 160, CV_32F);
        for (int p = 0; p < proto_pixels; p++) {
            float acc = 0.0f;
            for (int c = 0; c < 32; c++) {
                acc += coeff[c] * proto.data[(size_t)c * (size_t)proto_pixels + (size_t)p];
            }
            mask160.at<float>(p / 160, p % 160) = 1.0f / (1.0f + expf(-acc));
        }
        cv::Mat full;
        cv::resize(mask160, full, cv::Size(SIZE, SIZE), 0, 0, cv::INTER_LINEAR);
        cv::Rect roi(box.left, box.top, box.resized_w, box.resized_h);
        roi &= cv::Rect(0, 0, SIZE, SIZE);
        if (roi.width < 1 || roi.height < 1) {
            continue;
        }
        cv::Mat sized;
        cv::resize(full(roi), sized, cv::Size(image.cols, image.rows), 0, 0, cv::INTER_LINEAR);
        float x1 = cell(det, row, 0, cols);
        float y1 = cell(det, row, 1, cols);
        float x2 = cell(det, row, 2, cols);
        float y2 = cell(det, row, 3, cols);
        undo_box(x1, y1, x2, y2, box, image.cols, image.rows);
        int xa = std::max(0, (int)x1);
        int ya = std::max(0, (int)y1);
        int xb = std::min(image.cols, (int)x2);
        int yb = std::min(image.rows, (int)y2);
        int index = (int)std::lround(cell(det, row, 5, cols));
        cv::Scalar color = color_of(index);
        if (xb > xa && yb > ya) {
            for (int y = ya; y < yb; y++) {
                for (int x = xa; x < xb; x++) {
                    if (sized.at<float>(y, x) <= 0.5f) {
                        continue;
                    }
                    cv::Vec3b &pixel = image.at<cv::Vec3b>(y, x);
                    pixel[0] = (unsigned char)(pixel[0] * 0.45f + color[0] * 0.55f);
                    pixel[1] = (unsigned char)(pixel[1] * 0.45f + color[1] * 0.55f);
                    pixel[2] = (unsigned char)(pixel[2] * 0.45f + color[2] * 0.55f);
                }
            }
        }
        const char *name = (index >= 0 && index < COCO_N) ? COCO[index] : "object";
        char label[96];
        snprintf(label, sizeof label, "%s %.2f", name, cell(det, row, 4, cols));
        draw_box(image, x1, y1, x2, y2, label, color);
    }
}

struct Capture {
    cv::VideoCapture cap;
    std::string name;
    bool loop = false;
};

static Capture open_capture(const std::string &source) {
    Capture out;
    if (source.empty()) {
        for (int index = 0; index < 3; index++) {
            cv::VideoCapture cap(index);
            cv::Mat frame;
            if (cap.isOpened() && cap.read(frame) && !frame.empty()) {
                cap.set(cv::CAP_PROP_BUFFERSIZE, 1);
                out.cap = std::move(cap);
                out.name = "camera " + std::to_string(index);
                return out;
            }
        }
        throw std::runtime_error("no camera frame on /dev/video0, /dev/video1, or /dev/video2");
    }
    if (all_digits(source)) {
        int index = std::stoi(source);
        cv::VideoCapture cap(index);
        if (!cap.isOpened()) {
            throw std::runtime_error("camera " + source + " did not open");
        }
        cap.set(cv::CAP_PROP_BUFFERSIZE, 1);
        out.cap = std::move(cap);
        out.name = "camera " + source;
        return out;
    }
    std::string path = source;
    if (!is_file(path)) {
        std::string beside = root() + "/" + source;
        if (is_file(beside)) {
            path = beside;
        }
    }
    if (!is_file(path)) {
        throw std::runtime_error("video file not found: " + source);
    }
    cv::VideoCapture cap(path);
    if (!cap.isOpened()) {
        throw std::runtime_error("could not open video: " + path);
    }
    out.cap = std::move(cap);
    out.name = base_name(path);
    out.loop = true;
    return out;
}

static bool read_frame(Capture &capture, cv::Mat &frame) {
    if (capture.cap.read(frame) && !frame.empty()) {
        return true;
    }
    if (!capture.loop) {
        return false;
    }
    capture.cap.set(cv::CAP_PROP_POS_FRAMES, 0);
    return capture.cap.read(frame) && !frame.empty();
}

// Standing body, used only when a keypoint is missing. Deltas stay in the
// lifted skeleton, so a gap cannot move a limb into another coordinate space.
static const Vector3 REST[17] = {
    {0.00f, 1.62f, 0.00f},
    {-0.03f, 1.66f, 0.00f}, {0.03f, 1.66f, 0.00f},
    {-0.08f, 1.64f, 0.00f}, {0.08f, 1.64f, 0.00f},
    {-0.18f, 1.42f, 0.00f}, {0.18f, 1.42f, 0.00f},
    {-0.22f, 1.12f, 0.00f}, {0.22f, 1.12f, 0.00f},
    {-0.24f, 0.84f, 0.00f}, {0.24f, 0.84f, 0.00f},
    {-0.11f, 0.92f, 0.00f}, {0.11f, 0.92f, 0.00f},
    {-0.12f, 0.48f, 0.00f}, {0.12f, 0.48f, 0.00f},
    {-0.13f, 0.06f, 0.00f}, {0.13f, 0.06f, 0.00f},
};
static const int RIG_PARENT[17] = {
    -1, 0, 0, 1, 2,
    -1, -1,
    5, 6, 7, 8,
    -1, -1,
    11, 12, 13, 14,
};

struct Pose2D {
    float x[17];
    float y[17];
    float c[17];
    float score;
};

static Vector3 vec3(float x, float y, float z) {
    Vector3 v;
    v.x = x;
    v.y = y;
    v.z = z;
    return v;
}

static void bone(Vector3 a, Vector3 b, float radius, Color color) {
    float dx = a.x - b.x;
    float dy = a.y - b.y;
    float dz = a.z - b.z;
    if (dx * dx + dy * dy + dz * dz < 1.0e-6f) {
        return;
    }
    DrawCapsule(a, b, radius, 8, 6, color);
}

static void mid(Vector3 a, Vector3 b, Vector3 *out) {
    out->x = (a.x + b.x) * 0.5f;
    out->y = (a.y + b.y) * 0.5f;
    out->z = (a.z + b.z) * 0.5f;
}

static std::vector<Pose2D> people_from(const Tensor &det, const Letter &box) {
    std::vector<Pose2D> found;
    if (det.shape.size() != 3) {
        return found;
    }
    int rows = (int)det.shape[1];
    int cols = (int)det.shape[2];
    if (cols < 57) {
        return found;
    }
    for (int row = 0; row < rows; row++) {
        float score = cell(det, row, 4, cols);
        if (score < CONF) {
            continue;
        }
        Pose2D pose;
        pose.score = score;
        for (int k = 0; k < 17; k++) {
            float px = cell(det, row, 6 + k * 3, cols);
            float py = cell(det, row, 7 + k * 3, cols);
            float pc = cell(det, row, 8 + k * 3, cols);
            if (std::max(px, py) <= 1.5f) {
                px *= SIZE;
                py *= SIZE;
            }
            pose.x[k] = (px - box.left) / box.scale;
            pose.y[k] = (py - box.top) / box.scale;
            pose.c[k] = pc;
        }
        found.push_back(pose);
    }
    std::sort(found.begin(), found.end(), [](const Pose2D &a, const Pose2D &b) { return a.score > b.score; });
    if (found.size() > 3) {
        found.resize(3);
    }
    return found;
}

static void rest_from(int child, int parent, Vector3 *joint) {
    joint[child] = vec3(
        joint[parent].x + REST[child].x - REST[parent].x,
        joint[parent].y + REST[child].y - REST[parent].y,
        0.0f);
}

static void lift_person(const Pose2D &pose, float x_shift, Vector3 *joint) {
    bool have[17];
    int confident = 0;
    for (int i = 0; i < 17; i++) {
        have[i] = pose.c[i] >= KEYPOINT_CONF;
        if (have[i]) {
            confident += 1;
        }
    }
    if (confident < 4) {
        for (int i = 0; i < 17; i++) {
            joint[i] = REST[i];
            joint[i].x += x_shift;
        }
        return;
    }
    float ox = 0.0f;
    float oy = 0.0f;
    if (have[11] && have[12]) {
        ox = (pose.x[11] + pose.x[12]) * 0.5f;
        oy = (pose.y[11] + pose.y[12]) * 0.5f;
    } else if (have[5] && have[6]) {
        ox = (pose.x[5] + pose.x[6]) * 0.5f;
        oy = (pose.y[5] + pose.y[6]) * 0.5f;
    } else {
        for (int i = 0; i < 17; i++) {
            if (!have[i]) {
                continue;
            }
            ox += pose.x[i];
            oy += pose.y[i];
        }
        ox /= (float)confident;
        oy /= (float)confident;
    }
    float scale = 0.004f;
    if (have[11] && have[12] && have[5] && have[6]) {
        float dx = ox - (pose.x[5] + pose.x[6]) * 0.5f;
        float dy = oy - (pose.y[5] + pose.y[6]) * 0.5f;
        float pixels = std::sqrt(dx * dx + dy * dy);
        if (pixels > 4.0f) {
            scale = 0.50f / pixels;
        }
    } else {
        float ymin = 1.0e9f;
        float ymax = -1.0e9f;
        for (int i = 0; i < 17; i++) {
            if (!have[i]) {
                continue;
            }
            ymin = std::min(ymin, pose.y[i]);
            ymax = std::max(ymax, pose.y[i]);
        }
        if (ymax > ymin + 8.0f) {
            scale = 1.56f / (ymax - ymin);
        }
    }
    for (int i = 0; i < 17; i++) {
        if (!have[i]) {
            continue;
        }
        joint[i] = vec3((pose.x[i] - ox) * scale, (oy - pose.y[i]) * scale, 0.0f);
    }
    if (have[12] && !have[11]) {
        rest_from(11, 12, joint);
        have[11] = true;
    }
    if (have[11] && !have[12]) {
        rest_from(12, 11, joint);
        have[12] = true;
    }
    if (have[6] && !have[5]) {
        rest_from(5, 6, joint);
        have[5] = true;
    }
    if (have[5] && !have[6]) {
        rest_from(6, 5, joint);
        have[6] = true;
    }
    if (have[11] && !have[5]) {
        rest_from(5, 11, joint);
        have[5] = true;
    }
    if (have[12] && !have[6]) {
        rest_from(6, 12, joint);
        have[6] = true;
    }
    if (have[5] && !have[11]) {
        rest_from(11, 5, joint);
        have[11] = true;
    }
    if (have[6] && !have[12]) {
        rest_from(12, 6, joint);
        have[12] = true;
    }
    if (!have[0] && (have[5] || have[6])) {
        Vector3 chest;
        if (have[5] && have[6]) {
            mid(joint[5], joint[6], &chest);
        } else {
            chest = have[5] ? joint[5] : joint[6];
        }
        joint[0] = vec3(chest.x, chest.y + 0.20f, 0.0f);
        have[0] = true;
    }
    for (int pass = 0; pass < 6; pass++) {
        for (int i = 0; i < 17; i++) {
            int parent = RIG_PARENT[i];
            if (have[i] || parent < 0 || !have[parent]) {
                continue;
            }
            rest_from(i, parent, joint);
            have[i] = true;
        }
    }
    int known = 0;
    Vector3 mean = vec3(0.0f, 0.0f, 0.0f);
    Vector3 rest_mean = vec3(0.0f, 0.0f, 0.0f);
    for (int i = 0; i < 17; i++) {
        if (!have[i]) {
            continue;
        }
        mean.x += joint[i].x;
        mean.y += joint[i].y;
        rest_mean.x += REST[i].x;
        rest_mean.y += REST[i].y;
        known += 1;
    }
    if (known > 0) {
        mean.x /= (float)known;
        mean.y /= (float)known;
        rest_mean.x /= (float)known;
        rest_mean.y /= (float)known;
        for (int i = 0; i < 17; i++) {
            if (have[i]) {
                continue;
            }
            joint[i] = vec3(mean.x + REST[i].x - rest_mean.x, mean.y + REST[i].y - rest_mean.y, 0.0f);
        }
    }
    float ground = std::min(joint[15].y, joint[16].y);
    float lift = 0.06f - ground;
    for (int i = 0; i < 17; i++) {
        joint[i].y += lift;
        joint[i].x += x_shift;
    }
}

static void draw_character(const Vector3 *joint) {
    Color skin = {244, 196, 166, 255};
    Color shirt = {64, 112, 214, 255};
    Color pants = {42, 48, 78, 255};
    Color shoes = {46, 38, 34, 255};
    Vector3 neck;
    Vector3 waist;
    Vector3 chest;
    mid(joint[5], joint[6], &chest);
    mid(joint[11], joint[12], &waist);
    float dx = joint[0].x - chest.x;
    float dy = joint[0].y - chest.y;
    float dz = joint[0].z - chest.z;
    float span = std::sqrt(dx * dx + dy * dy + dz * dz);
    float reach = span > 0.12f ? (span - 0.09f) / span : 0.55f;
    neck = vec3(chest.x + dx * reach, chest.y + dy * reach, chest.z + dz * reach);
    bone(joint[11], joint[13], 0.048f, pants);
    bone(joint[13], joint[15], 0.040f, pants);
    bone(joint[12], joint[14], 0.048f, pants);
    bone(joint[14], joint[16], 0.040f, pants);
    bone(joint[11], joint[12], 0.055f, pants);
    bone(waist, chest, 0.090f, shirt);
    bone(joint[5], joint[6], 0.048f, shirt);
    bone(chest, neck, 0.040f, skin);
    bone(joint[5], joint[7], 0.046f, shirt);
    bone(joint[7], joint[9], 0.038f, skin);
    bone(joint[6], joint[8], 0.046f, shirt);
    bone(joint[8], joint[10], 0.038f, skin);
    DrawSphere(joint[0], 0.115f, skin);
    DrawSphere(joint[9], 0.034f, skin);
    DrawSphere(joint[10], 0.034f, skin);
    DrawSphere(joint[15], 0.040f, shoes);
    DrawSphere(joint[16], 0.040f, shoes);
}

static void render_character(RenderTexture2D stage, const Tensor &pose, const Letter &box, int frame_index) {
    std::vector<Pose2D> people = people_from(pose, box);
    if (people.empty()) {
        Pose2D idle;
        idle.score = 0.0f;
        for (int i = 0; i < 17; i++) {
            idle.x[i] = 0.0f;
            idle.y[i] = 0.0f;
            idle.c[i] = 0.0f;
        }
        people.push_back(idle);
    }
    float spin = frame_index * 0.018f;
    Camera3D camera;
    camera.position = vec3(std::sin(spin) * 4.2f, 1.35f, std::cos(spin) * 4.2f);
    camera.target = vec3(0.0f, 0.95f, 0.0f);
    camera.up = vec3(0.0f, 1.0f, 0.0f);
    camera.fovy = 36.0f;
    camera.projection = CAMERA_PERSPECTIVE;
    BeginTextureMode(stage);
    ClearBackground((Color){16, 18, 32, 255});
    BeginMode3D(camera);
    DrawGrid(16, 0.35f);
    int count = (int)people.size();
    for (int i = 0; i < count; i++) {
        Vector3 joint[17];
        float shift = ((float)i - (float)(count - 1) * 0.5f) * 1.7f;
        lift_person(people[i], shift, joint);
        draw_character(joint);
    }
    EndMode3D();
    DrawRectangle(0, 0, stage.texture.width, 32, (Color){16, 14, 28, 255});
    DrawText("character", 12, 8, 20, (Color){255, 220, 120, 255});
    EndTextureMode();
}

static std::vector<unsigned char> to_rgba(const cv::Mat &bgr) {
    std::vector<unsigned char> rgba((size_t)bgr.cols * (size_t)bgr.rows * 4);
    for (int y = 0; y < bgr.rows; y++) {
        const unsigned char *row = bgr.ptr<unsigned char>(y);
        for (int x = 0; x < bgr.cols; x++) {
            size_t i = ((size_t)y * (size_t)bgr.cols + (size_t)x) * 4;
            rgba[i] = row[x * 3 + 2];
            rgba[i + 1] = row[x * 3 + 1];
            rgba[i + 2] = row[x * 3];
            rgba[i + 3] = 255;
        }
    }
    return rgba;
}

static const char *present(int frames, const std::string &source) {
    void *ort_lib = dlopen((root() + "/lib/libonnxruntime.so.1.32.0").c_str(), RTLD_NOW | RTLD_LOCAL);
    if (!ort_lib) {
        snprintf(STATUS, sizeof STATUS, "onnxruntime: %s", dlerror());
        return STATUS;
    }
    auto get_base = (const OrtApiBase *(*)(void))dlsym(ort_lib, "OrtGetApiBase");
    if (!get_base) {
        snprintf(STATUS, sizeof STATUS, "onnxruntime: %s", dlerror());
        dlclose(ort_lib);
        return STATUS;
    }
    const OrtApi *api = get_base()->GetApi(ORT_API_VERSION);
    if (!api) {
        dlclose(ort_lib);
        return "onnxruntime: this header does not match the library";
    }
    OrtEnv *env = nullptr;
    check(api, api->CreateEnv(ORT_LOGGING_LEVEL_WARNING, "yolo", &env));
    OrtAllocator *alloc = nullptr;
    check(api, api->GetAllocatorWithDefaultOptions(&alloc));
    Net detect = load_net(api, env, alloc, root() + "/models/yolo26n.onnx");
    Net segment = load_net(api, env, alloc, root() + "/models/yolo26n-seg.onnx");
    Net pose = load_net(api, env, alloc, root() + "/models/yolo26n-pose.onnx");
    OrtMemoryInfo *memory = nullptr;
    check(api, api->CreateCpuMemoryInfo(OrtArenaAllocator, OrtMemTypeDefault, &memory));
    Capture capture = open_capture(source.empty() ? cmdline_source() : source);
    std::string title = "YOLO26  " + capture.name;
    InitWindow(1920, 360, title.c_str());
    SetTargetFPS(60);
    RenderTexture2D stage = LoadRenderTexture(480, 360);
    Texture2D texture = {};
    int seen = 0;
    double last_fps = 0.0;
    bool window = true;
    try {
        while (true) {
            cv::Mat frame;
            if (!read_frame(capture, frame)) {
                break;
            }
            if (frame.channels() == 4) {
                cv::cvtColor(frame, frame, cv::COLOR_BGRA2BGR);
            } else if (frame.channels() == 1) {
                cv::cvtColor(frame, frame, cv::COLOR_GRAY2BGR);
            }
            auto started = std::chrono::steady_clock::now();
            Letter box = letterbox(frame);
            std::vector<Tensor> det = run_net(api, memory, detect, box.blob);
            std::vector<Tensor> seg = run_net(api, memory, segment, box.blob);
            std::vector<Tensor> posed = run_net(api, memory, pose, box.blob);
            const Tensor *det_grid = grid_of(det, 6);
            const Tensor *seg_grid = grid_of(seg, 38);
            const Tensor *proto = proto_of(seg);
            const Tensor *pose_grid = grid_of(posed, 57);
            if (!det_grid || !seg_grid || !proto || !pose_grid) {
                throw std::runtime_error("a YOLO output had an unexpected shape");
            }
            cv::Mat panel_det = frame.clone();
            cv::Mat panel_seg = frame.clone();
            cv::Mat panel_pose = frame.clone();
            draw_detections(panel_det, *det_grid, box);
            draw_segments(panel_seg, *seg_grid, *proto, box);
            draw_pose(panel_pose, *pose_grid, box);
            double elapsed = std::chrono::duration<double>(std::chrono::steady_clock::now() - started).count();
            last_fps = elapsed > 0.0 ? 1.0 / elapsed : 0.0;
            char det_title[64];
            snprintf(det_title, sizeof det_title, "detection  %.1f fps", last_fps);
            paint_title(panel_det, det_title);
            paint_title(panel_seg, "segmentation");
            paint_title(panel_pose, "pose");
            cv::Mat view, seg_view, pose_view, canvas;
            cv::resize(panel_det, view, cv::Size(480, 360));
            cv::resize(panel_seg, seg_view, cv::Size(480, 360));
            cv::resize(panel_pose, pose_view, cv::Size(480, 360));
            cv::hconcat(std::vector<cv::Mat>{view, seg_view, pose_view}, canvas);
            std::vector<unsigned char> rgba = to_rgba(canvas);
            if (texture.id == 0) {
                Image image;
                image.data = rgba.data();
                image.width = canvas.cols;
                image.height = canvas.rows;
                image.mipmaps = 1;
                image.format = PIXELFORMAT_UNCOMPRESSED_R8G8B8A8;
                texture = LoadTextureFromImage(image);
            } else {
                UpdateTexture(texture, rgba.data());
            }
            render_character(stage, *pose_grid, box, seen);
            if (WindowShouldClose() || IsKeyPressed(KEY_Q)) {
                break;
            }
            BeginDrawing();
            ClearBackground(BLACK);
            DrawTexture(texture, 0, 0, WHITE);
            Rectangle source;
            source.x = 0.0f;
            source.y = 0.0f;
            source.width = (float)stage.texture.width;
            source.height = -(float)stage.texture.height;
            Rectangle dest;
            dest.x = 1440.0f;
            dest.y = 0.0f;
            dest.width = 480.0f;
            dest.height = 360.0f;
            Vector2 origin;
            origin.x = 0.0f;
            origin.y = 0.0f;
            DrawTexturePro(stage.texture, source, dest, origin, 0.0f, WHITE);
            EndDrawing();
            seen += 1;
            if (frames > 0 && seen >= frames) {
                break;
            }
        }
    } catch (...) {
        if (texture.id) {
            UnloadTexture(texture);
        }
        if (stage.id) {
            UnloadRenderTexture(stage);
        }
        if (window) {
            CloseWindow();
        }
        api->ReleaseMemoryInfo(memory);
        api->ReleaseSession(detect.session);
        api->ReleaseSession(segment.session);
        api->ReleaseSession(pose.session);
        api->ReleaseEnv(env);
        dlclose(ort_lib);
        throw;
    }
    if (texture.id) {
        UnloadTexture(texture);
    }
    if (stage.id) {
        UnloadRenderTexture(stage);
    }
    CloseWindow();
    capture.cap.release();
    api->ReleaseMemoryInfo(memory);
    api->ReleaseSession(detect.session);
    api->ReleaseSession(segment.session);
    api->ReleaseSession(pose.session);
    api->ReleaseEnv(env);
    dlclose(ort_lib);
    snprintf(STATUS, sizeof STATUS, "%s, frames %d, last %.1f fps, %d coco classes", capture.name.c_str(), seen, last_fps, COCO_N);
    return STATUS;
}

extern "C" const char *vision_run(int frames, const char *source) {
    try {
        return present(frames, source ? source : "");
    } catch (const std::exception &error) {
        snprintf(STATUS, sizeof STATUS, "yolo: %s", error.what());
        return STATUS;
    } catch (...) {
        return "yolo: failed";
    }
}
