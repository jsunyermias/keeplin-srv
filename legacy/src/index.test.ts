import { describe, it, before, after, beforeEach, afterEach } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from './server.js';
import { createToken, verifyToken } from './auth.js';
import type { User } from './types.js';
import WebSocket from 'ws';
import * as Y from 'yjs';
import * as sync from 'y-protocols/sync';
import { createEncoder, toUint8Array } from 'lib0/encoding';

const messageSync = 0;

async function waitForOpen(ws: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    ws.once('open', resolve);
    ws.once('error', reject);
  });
}

async function waitForMessage(ws: WebSocket): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    const handler = (data: Buffer) => {
      ws.off('message', handler);
      ws.off('error', errorHandler);
      resolve(new Uint8Array(data));
    };
    const errorHandler = (err: Error) => {
      ws.off('message', handler);
      ws.off('error', errorHandler);
      reject(err);
    };
    ws.on('message', handler);
    ws.on('error', errorHandler);
  });
}

async function waitForSyncMessage(ws: WebSocket): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    const handler = (data: Buffer) => {
      const message = new Uint8Array(data);
      if (message[0] === messageSync) {
        ws.off('message', handler);
        ws.off('error', errorHandler);
        resolve(message);
      }
    };
    const errorHandler = (err: Error) => {
      ws.off('message', handler);
      ws.off('error', errorHandler);
      reject(err);
    };
    ws.on('message', handler);
    ws.on('error', errorHandler);
  });
}

describe('auth', () => {
  it('creates and verifies a JWT token', async () => {
    const user: User = {
      id: 'u1',
      name: 'Alice',
      email: 'alice@keeplin.local',
      color: '#ff0000',
    };
    const token = await createToken(user);
    const decoded = await verifyToken(token);
    assert.equal(decoded.id, user.id);
    assert.equal(decoded.name, user.name);
  });
});

describe('api', () => {
  let server: ReturnType<typeof createServer>['server'];
  let baseUrl: string;
  let token: string;

  before(async () => {
    const created = createServer();
    server = created.server;
    await new Promise<void>((resolve) => server.listen(0, resolve));
    const address = server.address();
    if (address && typeof address === 'object') {
      baseUrl = `http://127.0.0.1:${address.port}`;
    } else {
      throw new Error('Server did not start');
    }

    const demoRes = await fetch(`${baseUrl}/api/auth/demo`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'Tester' }),
    });
    const demo = await demoRes.json();
    token = demo.token;
  });

  after(async () => {
    server.closeAllConnections?.();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it('returns health status', async () => {
    const res = await fetch(`${baseUrl}/health`);
    assert.equal(res.status, 200);
    const body = await res.json();
    assert.equal(body.status, 'ok');
  });

  it('creates and retrieves a document', async () => {
    const createRes = await fetch(`${baseUrl}/api/documents`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ title: 'Test doc' }),
    });
    assert.equal(createRes.status, 201);
    const doc = await createRes.json();
    assert.ok(doc.id);
    assert.equal(doc.title, 'Test doc');

    const getRes = await fetch(`${baseUrl}/api/documents/${doc.id}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    assert.equal(getRes.status, 200);
    const got = await getRes.json();
    assert.equal(got.id, doc.id);
  });
});

describe('websocket', () => {
  let server: ReturnType<typeof createServer>['server'];
  let baseUrl: string;
  let wsUrl: string;
  let token: string;

  beforeEach(async () => {
    const created = createServer();
    server = created.server;
    await new Promise<void>((resolve) => server.listen(0, resolve));
    const address = server.address();
    if (address && typeof address === 'object') {
      baseUrl = `http://127.0.0.1:${address.port}`;
      wsUrl = `ws://127.0.0.1:${address.port}`;
    } else {
      throw new Error('Server did not start');
    }

    const demoRes = await fetch(`${baseUrl}/api/auth/demo`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'Tester' }),
    });
    const demo = await demoRes.json();
    token = demo.token;
  });

  afterEach(async () => {
    server.closeAllConnections?.();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  async function createDocument(): Promise<string> {
    const docRes = await fetch(`${baseUrl}/api/documents`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: `Bearer ${token}`,
      },
      body: JSON.stringify({ title: 'WS test doc' }),
    });
    const doc = await docRes.json();
    return doc.id;
  }

  it('sends sync step 1 on connection', async () => {
    const docId = await createDocument();
    const ws = new WebSocket(`${wsUrl}/ws?token=${token}&docId=${docId}`);
    await waitForOpen(ws);
    const message = await waitForSyncMessage(ws);
    assert.equal(message[0], messageSync);
    ws.close();
  });

  it('replicates an update to another connected client', async () => {
    const docId = await createDocument();
    const ws1 = new WebSocket(`${wsUrl}/ws?token=${token}&docId=${docId}`);
    const ws2 = new WebSocket(`${wsUrl}/ws?token=${token}&docId=${docId}`);
    await Promise.all([waitForOpen(ws1), waitForOpen(ws2)]);

    // Registramos el listener para el update antes de enviarlo.
    const updatePromise = waitForSyncMessage(ws2);

    // Consumimos el sync step 1 inicial de ambos.
    await Promise.all([waitForSyncMessage(ws1), waitForSyncMessage(ws2)]);

    // ws1 envía un update Yjs.
    const localDoc = new Y.Doc();
    const text = localDoc.getText('content');
    text.insert(0, 'hello collaborative world');
    const update = Y.encodeStateAsUpdate(localDoc);

    const encoder = createEncoder();
    sync.writeUpdate(encoder, update);
    const syncPayload = toUint8Array(encoder);
    const fullMessage = new Uint8Array(syncPayload.length + 1);
    fullMessage[0] = messageSync;
    fullMessage.set(syncPayload, 1);
    ws1.send(fullMessage);

    // ws2 debería recibir el update replicado.
    const received = await updatePromise;
    assert.equal(received[0], messageSync);

    const remoteDoc = new Y.Doc();
    Y.applyUpdate(remoteDoc, received.slice(1));
    assert.equal(remoteDoc.getText('content').toString(), 'hello collaborative world');

    ws1.close();
    ws2.close();
  });
});
