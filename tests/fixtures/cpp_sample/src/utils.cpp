#include "utils.h"
#include <sstream>
#include <iomanip>

namespace utils {

bool isValid(const std::string& value) {
    return !value.empty();
}

int defaultTimeout() {
    return 30;
}

std::string formatDate(int timestamp) {
    std::stringstream ss;
    ss << timestamp;
    return ss.str();
}

} // namespace utils
