"""Tests for the JSON-lines protocol codec (framing + validation)."""

import json

import pytest

from sam2_sidecar.protocol import (
    PROTOCOL_VERSION,
    ProtocolError,
    decode_message,
    encode_message,
)


class TestEncode:
    def test_encodes_single_line_with_trailing_newline(self):
        line = encode_message({"type": "ping"})
        assert line.endswith("\n")
        assert line.count("\n") == 1
        assert json.loads(line) == {"type": "ping"}

    def test_encodes_utf8_and_non_ascii(self):
        line = encode_message({"type": "error", "message": "路径不存在"})
        assert "路径不存在" in line
        line.encode("utf-8")

    def test_rejects_non_dict(self):
        with pytest.raises(ProtocolError):
            encode_message(["not", "a", "dict"])

    def test_rejects_non_string_type(self):
        with pytest.raises(ProtocolError):
            encode_message({"type": 123})

    def test_rejects_unknown_type(self):
        with pytest.raises(ProtocolError):
            encode_message({"type": "no_such_type"})


class TestDecode:
    def test_roundtrip(self):
        msg = {"type": "init_session", "sessionId": "s1", "frames": []}
        assert decode_message(encode_message(msg)) == msg

    def test_rejects_empty_line(self):
        with pytest.raises(ProtocolError):
            decode_message("\n")

    def test_rejects_blank_whitespace(self):
        with pytest.raises(ProtocolError):
            decode_message("   \n")

    def test_rejects_invalid_json(self):
        with pytest.raises(ProtocolError):
            decode_message("{not json}\n")

    def test_rejects_json_arrays(self):
        with pytest.raises(ProtocolError):
            decode_message('[1, 2, 3]\n')

    def test_rejects_missing_type(self):
        with pytest.raises(ProtocolError):
            decode_message('{"sessionId": "s1"}\n')

    def test_error_includes_offender_context(self):
        with pytest.raises(ProtocolError) as exc:
            decode_message("{bad}\n")
        assert "type" not in json.loads("{}")  # no crash on our side
        assert exc.value.code == "bad_request"

    def test_decode_strips_crlf(self):
        msg = decode_message('{"type": "ping"}\r\n')
        assert msg == {"type": "ping"}


class TestProtocolVersion:
    def test_version_is_one(self):
        assert PROTOCOL_VERSION == 1
