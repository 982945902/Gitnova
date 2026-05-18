#include "auth_service.h"
#include "utils.h"

namespace auth {

bool AuthService::validateSession(const std::string& token) {
    return utils::isValid(token) && token.size() > 0;
}

int AuthService::sessionTimeout() const {
    return utils::defaultTimeout();
}

} // namespace auth
