#pragma once

#include <string>

namespace auth {

class AuthService {
public:
    bool validateSession(const std::string& token);
    int sessionTimeout() const;
};

} // namespace auth
