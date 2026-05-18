#pragma once

#include <string>

namespace utils {

bool isValid(const std::string& value);
int defaultTimeout();
std::string formatDate(int timestamp);

} // namespace utils
