import { Pool } from "pg";
const pool = new Pool();
export async function query(sql: string, params: unknown[] = []) {
  return pool.query(sql, params);
}
export function requireUser(req: Request) {
  return req.headers.get("x-user") ?? "anon";
}
