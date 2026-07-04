import express, { type Request, type Response, type NextFunction } from 'express';
import { createServer as createHttpServer } from 'node:http';
import { config } from './config.js';
import { createToken, requireAuth } from './auth.js';
import * as store from './store.js';
import { setupWebSocket } from './websocket.js';
import type { User, AuthenticatedRequest } from './types.js';

export function createServer() {
  const app = express();
  app.use(express.json());

  app.get('/health', (_req: Request, res: Response) => {
    res.json({ status: 'ok', env: config.nodeEnv });
  });

  // Genera un token de demo para facilitar pruebas. No usar en producción.
  app.post('/api/auth/demo', async (req: Request, res: Response) => {
    const body = req.body as { name?: string; color?: string };
    const user: User = {
      id: `user-${Date.now()}`,
      name: body.name ?? 'Demo User',
      email: `demo-${Date.now()}@keeplin.local`,
      color: body.color ?? '#3b82f6',
    };
    const token = await createToken(user);
    res.json({ token, user });
  });

  app.post('/api/documents', (req: Request, res: Response, next: NextFunction) => {
    requireAuth(req as AuthenticatedRequest, res, next);
  }, async (req: Request, res: Response) => {
    const authReq = req as AuthenticatedRequest;
    const body = authReq.body as { title?: string };
    const doc = await store.createDocument(body.title);
    res.status(201).json(doc);
  });

  app.get('/api/documents/:id', (req: Request, res: Response, next: NextFunction) => {
    requireAuth(req as AuthenticatedRequest, res, next);
  }, async (req: Request, res: Response) => {
    const doc = await store.getDocumentMeta(req.params.id);
    if (!doc) {
      res.status(404).json({ error: 'Document not found' });
      return;
    }
    res.json(doc);
  });

  const server = createHttpServer(app);
  setupWebSocket(server);

  return { app, server };
}
