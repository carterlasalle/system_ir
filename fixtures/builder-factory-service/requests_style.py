"""requests-family: fluent builder (Session.with_timeout), classmethod
factory (Session.from_config), module factory (create_session), and
module-level globals (mutable STATE)."""

DEFAULT_TIMEOUT = 30
_session_cache = {}


class Session:
    """An HTTP session; configured fluently."""

    def __init__(self, cfg=None):
        self._timeout = DEFAULT_TIMEOUT
        self._headers = {}
        if cfg:
            self._timeout = cfg.get("timeout", DEFAULT_TIMEOUT)

    def with_timeout(self, seconds):
        self._timeout = seconds
        return self

    def with_headers(self, headers):
        self._headers.update(headers)
        return self

    def request(self, method, url):
        return PreparedRequest(method, url)

    @classmethod
    def from_config(cls, cfg):
        return cls(cfg)


class PreparedRequest:
    def __init__(self, method, url):
        self.method = method
        self.url = url


def create_session(cfg=None):
    """Build a configured session."""
    return Session(cfg)
