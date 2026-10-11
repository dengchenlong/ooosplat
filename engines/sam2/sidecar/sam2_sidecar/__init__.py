"""SAM2 sidecar: JSON-lines subprocess protocol for OOOSplat.

Privacy constraint: all inference runs locally. No cloud APIs.
"""

PROTOCOL_VERSION = 1

# Request types (client -> sidecar)
REQ_HELLO = "hello"
REQ_PING = "ping"
REQ_INIT_SESSION = "init_session"
REQ_ADD_PROMPT = "add_prompt"
REQ_PROPAGATE = "propagate"
REQ_SHUTDOWN = "shutdown"

# Response types (sidecar -> client)
RES_HELLO = "hello"
RES_PONG = "pong"
RES_SESSION_READY = "session_ready"
RES_PROMPT_ACCEPTED = "prompt_accepted"
RES_MASK = "mask"
RES_PROPAGATE_DONE = "propagate_done"
RES_BYE = "bye"
RES_ERROR = "error"
