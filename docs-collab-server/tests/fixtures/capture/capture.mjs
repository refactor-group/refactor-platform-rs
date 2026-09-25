// Capture real Hocuspocus wire bytes for the docs-collab-server tests.
//
// We do NOT import @hocuspocus/provider's per-message classes directly: the
// package's "exports" field hides them. Instead each builder below mirrors
// the OutgoingMessage class body line-for-line, calling the same underlying
// lib0 + y-protocols + @hocuspocus/common primitives that the provider's
// classes call. The resulting bytes are byte-identical to what the provider
// puts on the wire. See ../../node_modules/@hocuspocus/provider/src/
// OutgoingMessages/*.ts for the reference bodies.

import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import * as Y from "yjs";
import { Awareness, encodeAwarenessUpdate } from "y-protocols/awareness";
import * as syncProtocol from "y-protocols/sync";
import * as encoding from "lib0/encoding";

import {
  writeAuthentication,
  writeAuthenticated,
  writePermissionDenied,
} from "@hocuspocus/common";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = resolve(here, "..");
mkdirSync(outDir, { recursive: true });

const DOC_NAME = "refactor-coaching.alice-bob.aaaaaaaa-v0";
const TOKEN = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ0ZXN0In0.fake";
const STATELESS_PAYLOAD = "hello stateless";
const SCOPE = "read-write";
const DENY_REASON = "not authorized";

// Outer message types, mirror of provider's enum (types.ts).
const MessageType = {
  Sync: 0,
  Awareness: 1,
  Auth: 2,
  QueryAwareness: 3,
  Stateless: 5,
  CLOSE: 7,
  SyncStatus: 8,
};

const fixtures = [];

function frame(name, build, meta) {
  const enc = encoding.createEncoder();
  encoding.writeVarString(enc, DOC_NAME);
  build(enc);
  const bytes = encoding.toUint8Array(enc);
  const file = `${name}.bin`;
  writeFileSync(resolve(outDir, file), Buffer.from(bytes));
  fixtures.push({ file, doc_name: DOC_NAME, ...meta });
}

// --- Client -> Server ---------------------------------------------------------

// A doc with a single root text "t" containing "hi" so SyncStep2/Update have
// real, non-trivial payloads.
const baseDoc = new Y.Doc();
baseDoc.getText("t").insert(0, "hi");

frame(
  "sync_step1",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Sync);
    syncProtocol.writeSyncStep1(enc, baseDoc);
  },
  { kind: "SyncStep1", outer_tag: MessageType.Sync },
);

frame(
  "sync_step2",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Sync);
    syncProtocol.writeSyncStep2(enc, baseDoc);
  },
  { kind: "SyncStep2", outer_tag: MessageType.Sync },
);

frame(
  "update",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Sync);
    syncProtocol.writeUpdate(enc, Y.encodeStateAsUpdate(baseDoc));
  },
  { kind: "Update", outer_tag: MessageType.Sync },
);

const awareness = new Awareness(baseDoc);
awareness.setLocalState({ user: { name: "alice", color: "#abc" } });
frame(
  "awareness",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Awareness);
    encoding.writeVarUint8Array(
      enc,
      encodeAwarenessUpdate(awareness, [awareness.clientID]),
    );
  },
  { kind: "Awareness", outer_tag: MessageType.Awareness },
);

frame(
  "awareness_query",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.QueryAwareness);
  },
  { kind: "AwarenessQuery", outer_tag: MessageType.QueryAwareness },
);

frame(
  "auth_token",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Auth);
    writeAuthentication(enc, TOKEN);
  },
  { kind: "AuthToken", outer_tag: MessageType.Auth, payload_string: TOKEN },
);

frame(
  "stateless",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Stateless);
    encoding.writeVarString(enc, STATELESS_PAYLOAD);
  },
  {
    kind: "Stateless",
    outer_tag: MessageType.Stateless,
    payload_string: STATELESS_PAYLOAD,
  },
);

frame(
  "close",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.CLOSE);
  },
  { kind: "Close", outer_tag: MessageType.CLOSE },
);

// --- Server -> Client ---------------------------------------------------------

frame(
  "authenticated",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Auth);
    writeAuthenticated(enc, SCOPE);
  },
  {
    kind: "Authenticated",
    outer_tag: MessageType.Auth,
    payload_string: SCOPE,
  },
);

frame(
  "permission_denied",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.Auth);
    writePermissionDenied(enc, DENY_REASON);
  },
  {
    kind: "PermissionDenied",
    outer_tag: MessageType.Auth,
    payload_string: DENY_REASON,
  },
);

frame(
  "sync_status",
  (enc) => {
    encoding.writeVarUint(enc, MessageType.SyncStatus);
    // SyncStatus payload is a varInt 0|1.
    encoding.writeVarInt(enc, 1);
  },
  {
    kind: "SyncStatus",
    outer_tag: MessageType.SyncStatus,
    payload_bool: true,
  },
);

writeFileSync(
  resolve(outDir, "manifest.json"),
  JSON.stringify(
    {
      source:
        "@hocuspocus/common 2.15.3 + y-protocols 1.0.6 + yjs 13.6.27 + lib0 0.2.114",
      doc_name: DOC_NAME,
      auth_token: TOKEN,
      stateless_payload: STATELESS_PAYLOAD,
      authenticated_scope: SCOPE,
      permission_denied_reason: DENY_REASON,
      fixtures,
    },
    null,
    2,
  ) + "\n",
);

console.log(`wrote ${fixtures.length} fixtures + manifest.json to ${outDir}`);
