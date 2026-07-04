import * as fs from 'node:fs/promises';
import * as path from 'node:path';
import * as Y from 'yjs';
import { v4 as uuid } from 'uuid';
import { config } from './config.js';
import type { Document } from './types.js';

async function ensureDataDir(): Promise<void> {
  await fs.mkdir(config.dataDir, { recursive: true });
}

function metaPath(docId: string): string {
  return path.join(config.dataDir, `${docId}.meta.json`);
}

function updatesPath(docId: string): string {
  return path.join(config.dataDir, `${docId}.updates.json`);
}

export async function documentExists(docId: string): Promise<boolean> {
  try {
    await fs.access(metaPath(docId));
    return true;
  } catch {
    return false;
  }
}

export async function createDocument(title = 'Untitled document'): Promise<Document> {
  await ensureDataDir();
  const now = new Date().toISOString();
  const doc: Document = {
    id: uuid(),
    title,
    createdAt: now,
    updatedAt: now,
  };
  await fs.writeFile(metaPath(doc.id), JSON.stringify(doc, null, 2));
  await fs.writeFile(updatesPath(doc.id), JSON.stringify([]));
  return doc;
}

export async function getDocumentMeta(docId: string): Promise<Document | null> {
  try {
    const raw = await fs.readFile(metaPath(docId), 'utf-8');
    return JSON.parse(raw) as Document;
  } catch {
    return null;
  }
}

export async function updateDocumentMeta(docId: string, patch: Partial<Document>): Promise<Document | null> {
  const meta = await getDocumentMeta(docId);
  if (!meta) return null;
  const updated = { ...meta, ...patch, updatedAt: new Date().toISOString() };
  await fs.writeFile(metaPath(docId), JSON.stringify(updated, null, 2));
  return updated;
}

export async function loadYDoc(docId: string): Promise<Y.Doc> {
  const doc = new Y.Doc();
  try {
    const raw = await fs.readFile(updatesPath(docId), 'utf-8');
    const updates = JSON.parse(raw) as string[];
    for (const update of updates) {
      Y.applyUpdate(doc, Buffer.from(update, 'base64'));
    }
  } catch {
    // Si no existe el archivo de updates, devolvemos un documento vacío.
  }
  return doc;
}

export async function saveUpdate(docId: string, update: Uint8Array): Promise<void> {
  await ensureDataDir();
  const file = updatesPath(docId);
  let updates: string[] = [];
  try {
    const raw = await fs.readFile(file, 'utf-8');
    updates = JSON.parse(raw) as string[];
  } catch {
    // El archivo no existe todavía.
  }
  updates.push(Buffer.from(update).toString('base64'));
  await fs.writeFile(file, JSON.stringify(updates));
}
