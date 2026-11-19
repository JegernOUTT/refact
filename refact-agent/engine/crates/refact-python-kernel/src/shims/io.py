"""Refact shim for the stdlib io module (Monty): in-memory text and byte streams.

Has StringIO, BytesIO, open and the SEEK_* constants. Missing: TextIOWrapper,
BufferedReader and the other stream classes; StringIO does not translate newlines
(newline=None keeps '\r\n'); BytesIO has no readline or line iteration.
"""

_builtin_open = open

SEEK_SET = 0
SEEK_CUR = 1
SEEK_END = 2
DEFAULT_BUFFER_SIZE = 8192

UnsupportedOperation = OSError


class StringIO:
    def __init__(self, initial_value="", newline="\n"):
        text = "" if initial_value is None else str(initial_value)
        self._parts = [text] if text else []
        self._size = len(text)
        self._pos = 0
        self.closed = False

    def _check(self):
        if self.closed:
            raise ValueError("I/O operation on closed file.")

    def _text(self):
        if len(self._parts) > 1:
            self._parts = ["".join(self._parts)]
        return self._parts[0] if self._parts else ""

    def getvalue(self):
        self._check()
        return self._text()

    def write(self, s):
        self._check()
        if not isinstance(s, str):
            raise TypeError("string argument expected, got '" + type(s).__name__ + "'")
        if self._pos == self._size:
            self._parts.append(s)
            self._size += len(s)
        else:
            v = self._text()
            if self._pos > len(v):
                v = v + "\0" * (self._pos - len(v))
            v = v[:self._pos] + s + v[self._pos + len(s):]
            self._parts = [v]
            self._size = len(v)
        self._pos += len(s)
        return len(s)

    def writelines(self, lines):
        for line in lines:
            self.write(line)

    def read(self, size=-1):
        self._check()
        v = self._text()
        end = len(v) if size is None or size < 0 else self._pos + size
        out = v[self._pos:end]
        self._pos += len(out)
        return out

    def readline(self, size=-1):
        self._check()
        v = self._text()
        end = v.find("\n", self._pos)
        end = len(v) if end < 0 else end + 1
        if size is not None and size >= 0:
            end = min(end, self._pos + size)
        out = v[self._pos:end]
        self._pos += len(out)
        return out

    def readlines(self, hint=-1):
        out = []
        while True:
            line = self.readline()
            if not line:
                return out
            out.append(line)

    def __iter__(self):
        self._check()
        return self

    def __next__(self):
        line = self.readline()
        if not line:
            raise StopIteration
        return line

    def seek(self, pos, whence=0):
        self._check()
        if whence == 0 and pos < 0:
            raise ValueError("Negative seek position " + str(pos))
        if whence == 1:
            pos = self._pos + pos
        elif whence == 2:
            pos = self._size + pos
        self._pos = max(0, pos)
        return self._pos

    def tell(self):
        self._check()
        return self._pos

    def truncate(self, size=None):
        self._check()
        size = self._pos if size is None else size
        self._parts = [self._text()[:size]]
        self._size = len(self._parts[0])
        return size

    def readable(self):
        self._check()
        return True

    def writable(self):
        self._check()
        return True

    def seekable(self):
        self._check()
        return True

    def flush(self):
        self._check()
        return None

    def close(self):
        self.closed = True

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False


class BytesIO:
    def __init__(self, initial_bytes=b""):
        self._value = b"" if initial_bytes is None else bytes(initial_bytes)
        self._pos = 0
        self.closed = False

    def _check(self):
        if self.closed:
            raise ValueError("I/O operation on closed file.")

    def getvalue(self):
        self._check()
        return self._value

    def write(self, b):
        self._check()
        b = bytes(b)
        v = self._value
        self._value = v[:self._pos] + b + v[self._pos + len(b):]
        self._pos += len(b)
        return len(b)

    def read(self, size=-1):
        self._check()
        v = self._value
        end = len(v) if size is None or size < 0 else self._pos + size
        out = v[self._pos:end]
        self._pos += len(out)
        return out

    def seek(self, pos, whence=0):
        self._check()
        if whence == 0 and pos < 0:
            raise ValueError("negative seek value " + str(pos))
        if whence == 1:
            pos = self._pos + pos
        elif whence == 2:
            pos = len(self._value) + pos
        self._pos = max(0, pos)
        return self._pos

    def tell(self):
        self._check()
        return self._pos

    def close(self):
        self.closed = True

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False


def open(file, mode="r", buffering=-1, encoding=None, errors=None, newline=None, closefd=True, opener=None):
    if errors is not None:
        raise TypeError("io.open(): errors= is not supported here; read bytes and decode instead")
    return _builtin_open(file, mode, encoding=encoding, newline=newline)
