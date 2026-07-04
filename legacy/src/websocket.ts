import type { Server as HttpServer } from 'node:http';
import { WebSocketServer, WebSocket } from 'ws';
import * as Y from 'yjs';
import * as sync from 'y-protocols/sync';
import * as awarenessProtocol from 'y-protocols/awareness';
import { Encoder, createEncoder, length, toUint8Array } from 'lib0/encoding';
import { Decoder, createDecoder } from 'lib0/decoding';
import type { User } from './types.js';
import { verifyToken, parseTokenFromUrl, parseDocIdFromUrl } from './auth.js';
import * as store from './store.js';

const messageSync = 0;
const messageAwareness = 1;

interface Connection {
  ws: WebSocket;
  user: User;
  docId: string;
  awarenessClientIds: Set<number>;
}

const rooms = new Map<string, Set<WebSocket>>();
const docs = new Map<string, Y.Doc>();
const awarenesses = new Map<string, awarenessProtocol.Awareness>();
const connections = new Map<WebSocket, Connection>();

function getOrCreateRoom(docId: string): Set<WebSocket> {
  if (!rooms.has(docId)) {
    rooms.set(docId, new Set());
  }
  return rooms.get(docId)!;
}

async function getOrCreateDoc(docId: string): Promise<Y.Doc> {
  if (!docs.has(docId)) {
    const doc = await store.loadYDoc(docId);
    docs.set(docId, doc);

    doc.on('update', (update, origin) => {
      const clients = getOrCreateRoom(docId);
      const message = new Uint8Array(update.length + 1);
      message[0] = messageSync;
      message.set(update, 1);

      for (const client of clients) {
        if (client !== origin && client.readyState === WebSocket.OPEN) {
          client.send(message);
        }
      }

      store.saveUpdate(docId, update).catch((err) => {
        console.error(`Failed to save update for ${docId}:`, err);
      });

      store.updateDocumentMeta(docId, { updatedAt: new Date().toISOString() }).catch((err) => {
        console.error(`Failed to update meta for ${docId}:`, err);
      });
    });
  }
  return docs.get(docId)!;
}

function getOrCreateAwareness(docId: string): awarenessProtocol.Awareness {
  if (!awarenesses.has(docId)) {
    const doc = docs.get(docId);
    if (!doc) throw new Error(`Doc ${docId} must be loaded before creating awareness`);
    const awareness = new awarenessProtocol.Awareness(doc);
    awareness.on('update', ({ added, updated, removed }: { added: number[]; updated: number[]; removed: number[] }) => {
      const changedClients = [...added, ...updated, ...removed];
      const clients = getOrCreateRoom(docId);
      const update = awarenessProtocol.encodeAwarenessUpdate(awareness, changedClients);
      const message = new Uint8Array(update.length + 1);
      message[0] = messageAwareness;
      message.set(update, 1);
      for (const client of clients) {
        if (client.readyState === WebSocket.OPEN) {
          client.send(message);
        }
      }
    });
    awarenesses.set(docId, awareness);
  }
  return awarenesses.get(docId)!;
}

export function setupWebSocket(server: HttpServer): void {
  const wss = new WebSocketServer({ server, path: '/ws' });

  wss.on('connection', async (ws, req) => {
    try {
      const token = parseTokenFromUrl(req.url ?? '');
      if (!token) {
        ws.close(1008, 'Missing token');
        return;
      }

      const user = await verifyToken(token);
      const docId = parseDocIdFromUrl(req.url ?? '');
      if (!docId) {
        ws.close(1008, 'Missing docId');
        return;
      }

      const exists = await store.documentExists(docId);
      if (!exists) {
        ws.close(1008, 'Document not found');
        return;
      }

      const doc = await getOrCreateDoc(docId);
      const room = getOrCreateRoom(docId);
      const awareness = getOrCreateAwareness(docId);

      const connection: Connection = {
        ws,
        user,
        docId,
        awarenessClientIds: new Set(),
      };
      connections.set(ws, connection);
      room.add(ws);

      // Enviamos el sync step 1: state vector del servidor.
      const encoder = createEncoder();
      sync.writeSyncStep1(encoder, doc);
      const syncMessage = toUint8Array(encoder);
      const fullMessage = new Uint8Array(syncMessage.length + 1);
      fullMessage[0] = messageSync;
      fullMessage.set(syncMessage, 1);
      ws.send(fullMessage);

      ws.on('message', (data) => {
        try {
          const message = new Uint8Array(data as ArrayBuffer);
          if (message.length === 0) return;

          const messageType = message[0];
          const payload = message.slice(1);

          if (messageType === messageSync) {
            const decoder = createDecoder(payload);
            const encoder = createEncoder();
            sync.readSyncMessage(decoder, encoder, doc, ws);

            if (length(encoder) > 0) {
              const reply = toUint8Array(encoder);
              const replyMessage = new Uint8Array(reply.length + 1);
              replyMessage[0] = messageSync;
              replyMessage.set(reply, 1);
              ws.send(replyMessage);
            }
          } else if (messageType === messageAwareness) {
            const previousStates = awareness.getStates();
            awarenessProtocol.applyAwarenessUpdate(awareness, payload, ws);
            const currentStates = awareness.getStates();
            for (const clientId of currentStates.keys()) {
              if (!previousStates.has(clientId)) {
                connection.awarenessClientIds.add(clientId);
              }
            }
          }
        } catch (err) {
          console.error('Error handling WebSocket message:', err);
        }
      });

      ws.on('close', () => {
        room.delete(ws);
        connections.delete(ws);
        const clientIds = [...connection.awarenessClientIds];
        if (clientIds.length > 0) {
          awarenessProtocol.removeAwarenessStates(awareness, clientIds, 'server-close');
        }
        if (room.size === 0) {
          awarenesses.delete(docId);
          docs.delete(docId);
          rooms.delete(docId);
        }
      });

      ws.on('error', (err) => {
        console.error('WebSocket error:', err);
      });
    } catch (err) {
      console.error('WebSocket connection error:', err);
      ws.close(1008, 'Unauthorized');
    }
  });
}
