"""Use the current user's identity without administrator grants or privileges."""
from contextlib import contextmanager


@contextmanager
def restricted_identity():
    import win32api
    import win32con
    import win32security as security
    token = security.OpenProcessToken(win32api.GetCurrentProcess(),
                                     win32con.TOKEN_DUPLICATE | win32con.TOKEN_QUERY)
    restricted = None
    try:
        admins = security.CreateWellKnownSid(security.WinBuiltinAdministratorsSid, None)
        restricted = security.CreateRestrictedToken(token, 1, [(admins, 0)], [], [])
        security.ImpersonateLoggedOnUser(restricted)
        try:
            yield
        finally:
            security.RevertToSelf()
    finally:
        if restricted:
            restricted.Close()
        token.Close()
