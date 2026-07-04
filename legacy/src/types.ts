import type { Request } from 'express';

export interface User {
  id: string;
  name: string;
  email: string;
  color: string;
}

export interface Document {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
}

export interface AuthenticatedRequest extends Request {
  user?: User;
}
