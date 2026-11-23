#include <cmath>

extern "C" {

double sine_wave(double turns) {
    return std::sin(turns * 6.283185307179586);
}

}
