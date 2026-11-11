#!/usr/bin/env python3
"""Local stand-in for the account services an online-mode server and client talk to, for lab
tests without real accounts. Never use it outside 127.0.0.1.

  fake-session-server.py <port>

Endpoints (the paths of the real services; the server finds them through authlib's
-Dminecraft.api.session.host / services.host / profiles.host properties, the client through
ghast-auth's Endpoints::with_base_url):
  POST /session/minecraft/join          client: {accessToken, selectedProfile, serverId} -> 204
  GET  /session/minecraft/hasJoined     server: ?username=&serverId=[&ip=] -> the profile, or 204
  GET  /session/minecraft/profile/<id>  server: the profile (no properties)
  GET  /publickeys                      server: this stand-in's signing key (player certificates)
  POST /player/certificates             client: a fresh profile key pair signed with that key
  POST /minecraft/profile/lookup/bulk/byname, GET /minecraft/profile/lookup/name/<n>: 404 / []

Access tokens are "fake:<profile id without dashes>"; anything else is rejected (403). Every
request is logged to stdout as one line (the lab script checks them).
"""
import base64
import datetime
import json
import sys
import threading
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa

LOCK = threading.Lock()
JOINS = {}  # serverId -> profile id (hex, no dashes)
SERVICES_KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)
SERVICES_DER = SERVICES_KEY.public_key().public_bytes(
    serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)


def pem(tag, der):
    body = base64.encodebytes(der).decode().replace("\n", "")
    lines = "\n".join(body[i:i + 76] for i in range(0, len(body), 76))
    return f"-----BEGIN {tag}-----\n{lines}\n-----END {tag}-----\n"


def iso(t):
    return t.strftime("%Y-%m-%dT%H:%M:%S.%f") + "Z"


def certificates(profile_hex):
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    spki = key.public_key().public_bytes(
        serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    pkcs8 = key.private_bytes(serialization.Encoding.DER, serialization.PrivateFormat.PKCS8,
                              serialization.NoEncryption())
    now = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
    expires = now + datetime.timedelta(hours=48)
    expires_ms = int((expires - datetime.datetime(1970, 1, 1)).total_seconds() * 1000)
    # ProfilePublicKey.Data.signedPayload: profile id (16 bytes), expiry ms (8), the DER key.
    payload = uuid.UUID(hex=profile_hex).bytes + expires_ms.to_bytes(8, "big", signed=True) + spki
    sig = SERVICES_KEY.sign(payload, padding.PKCS1v15(), hashes.SHA1())
    # The real service prints the expiry with microseconds; Instant.parse reads it to the ms we signed.
    expires = datetime.datetime(1970, 1, 1) + datetime.timedelta(milliseconds=expires_ms)
    return {
        "keyPair": {"privateKey": pem("RSA PRIVATE KEY", pkcs8),
                    "publicKey": pem("RSA PUBLIC KEY", spki)},
        "publicKeySignature": base64.b64encode(sig).decode(),
        "publicKeySignatureV2": base64.b64encode(sig).decode(),
        "expiresAt": iso(expires),
        "refreshedAfter": iso(now + datetime.timedelta(hours=40)),
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        pass

    def reply(self, code, body=None):
        data = b"" if body is None else json.dumps(body).encode()
        self.send_response(code)
        if body is not None:
            self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
        print(f"{self.command} {urlparse(self.path).path} -> {code}", flush=True)

    def token_profile(self):
        auth = self.headers.get("Authorization", "")
        tok = auth[7:] if auth.startswith("Bearer ") else ""
        return tok[5:] if tok.startswith("fake:") and len(tok) == 37 else None

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def do_POST(self):
        u = urlparse(self.path)
        raw = self.body()
        if u.path == "/session/minecraft/join":
            req = json.loads(raw or b"{}")
            tok, prof = req.get("accessToken", ""), req.get("selectedProfile", "")
            if tok != "fake:" + prof:
                return self.reply(403, {"error": "ForbiddenOperationException", "errorMessage": "Invalid token"})
            with LOCK:
                JOINS[req.get("serverId", "")] = prof
            return self.reply(204)
        if u.path == "/player/certificates":
            prof = self.token_profile()
            if prof is None:
                return self.reply(401)
            return self.reply(200, certificates(prof))
        if u.path.startswith("/minecraft/profile/lookup"):
            return self.reply(200, [])
        return self.reply(404)

    def do_GET(self):
        u = urlparse(self.path)
        q = parse_qs(u.query)
        if u.path == "/session/minecraft/hasJoined":
            name, server_id = q.get("username", [""])[0], q.get("serverId", [""])[0]
            with LOCK:
                prof = JOINS.pop(server_id, None)
            if prof is None:
                return self.reply(204)
            return self.reply(200, {"id": prof, "name": name, "properties": []})
        if u.path.startswith("/session/minecraft/profile/"):
            prof = u.path.rsplit("/", 1)[-1]
            return self.reply(200, {"id": prof, "name": "", "properties": []})
        if u.path == "/publickeys":
            k = {"publicKey": base64.b64encode(SERVICES_DER).decode()}
            return self.reply(200, {"profilePropertyKeys": [k], "playerCertificateKeys": [k],
                                    "authenticationKeys": [k]})
        return self.reply(404)


def main():
    port = int(sys.argv[1])
    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"fake session server on 127.0.0.1:{port}", flush=True)
    srv.serve_forever()


if __name__ == "__main__":
    main()
