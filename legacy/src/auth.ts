import { SignJWT, jwtVerify } from 'jose';
import type { Request, Response, NextFunction } from 'express';
import type { User, AuthenticatedRequest } from './types.js';
import { config } from './config.js';

const encoder = new TextEncoder();

export async function createToken(user: User): Promise<string> {
  return new SignJWT({ ...user })
    .setProtectedHeader({ alg: 'HS256' })
    .setIssuedAt()
    .setExpirationTime('24h')
    .sign(encoder.encode(config.jwtSecret));
}

export async function verifyToken(token: string): Promise<User> {
  const { payload } = await jwtVerify(token, encoder.encode(config.jwtSecret), {
    algorithms: ['HS256'],
  });
  return payload as unknown as User;
}

export function requireAuth(
  req: AuthenticatedRequest,
  res: Response,
  next: NextFunction,
): void {
  const authHeader = req.headers['authorization'];
  const token = typeof authHeader === 'string' && authHeader.startsWith('Bearer ') ? authHeader.slice(7) : undefined;

  if (!token) {
    res.status(401).json({ error: 'Missing token' });
    return;
  }

  verifyToken(token)
    .then((user) => {
      req.user = user;
      next();
    })
    .catch(() => {
      res.status(401).json({ error: 'Invalid token' });
    });
}

export function parseTokenFromUrl(url: string): string | undefined {
  const parsed = new URL(url, 'http://localhost');
  return parsed.searchParams.get('token') ?? undefined;
}

export function parseDocIdFromUrl(url: string): string | undefined {
  const parsed = new URL(url, 'http://localhost');
  return parsed.searchParams.get('docId') ?? undefined;
}
