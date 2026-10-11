"""JSON-lines protocol codec.

Framing: one UTF-8 JSON object per line, terminated by ``\\n`` (a trailing
``\\r`` is tolerated on input). Every message must be a JSON object with a
string ``type`` field from the protocol vocabulary.
"""

import json
from typing import Any

from . import (
    REQ_ADD_PROMPT,
    REQ_HELLO,
    REQ_INIT_SESSION,
    REQ_PING,
    REQ_PROPAGATE,
    REQ_SHUTDOWN,
)

PROTOCOL_VERSION = 1

KNOWN_TYPES = frozenset(
    {
        REQ_HELLO,
        REQ_PING,
        REQ_INIT_SESSION,
        REQ_ADD_PROMPT,
        REQ_PROPAGATE,
        REQ_SHUTDOWN,
        # response types are emitted by this sidecar but round-trip through
        # the same codec in tests and client code
        "hello",
        "pong",
        "session_ready",
        "prompt_accepted",
        "mask",
        "propagate_done",
        "bye",
        "error",
    }
)


class ProtocolError(ValueError):
    """A message violated the JSON-lines protocol contract."""

    def __init__(self, message: str, code: str = "bad_request"):
        super().__init__(message)
        self.code = code


def encode_message(message: dict[str, Any]) -> str:
    """Serialize one protocol message to a single newline-terminated line."""
    _validate_object(message)
    return json.dumps(message, ensure_ascii=False) + "\n"


def decode_message(line: str) -> dict[str, Any]:
    """Parse one protocol message from a raw input line."""
    stripped = line.strip()
    if not stripped:
        raise ProtocolError("empty protocol line")
    try:
        message = json.loads(stripped)
    except json.JSONDecodeError as exc:
        raise ProtocolError(f"invalid JSON: {exc.msg}") from exc
    _validate_object(message)
    return message


def _validate_object(message: Any) -> None:
    if not isinstance(message, dict):
        raise ProtocolError("protocol message must be a JSON object")
    msg_type = message.get("type")
    if not isinstance(msg_type, str) or not msg_type:
        raise ProtocolError("protocol message requires a non-empty string 'type'")
    if msg_type not in KNOWN_TYPES:
        raise ProtocolError(f"unknown protocol message type: {msg_type!r}")
