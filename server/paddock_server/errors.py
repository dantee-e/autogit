class AppError(Exception):
    """Domain error. `message` is user-facing (PT-BR) and shown by the client."""
    status_code = 400

    def __init__(self, message: str):
        super().__init__(message)
        self.message = message


class Invalid(AppError):
    status_code = 400


class Unauthorized(AppError):
    status_code = 401


class Forbidden(AppError):
    status_code = 403


class NotFound(AppError):
    status_code = 404


class Conflict(AppError):
    status_code = 409


class TooManyRequests(AppError):
    status_code = 429
