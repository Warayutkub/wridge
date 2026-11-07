import { query, requireUser } from "@/lib/db";
export async function GET(req: Request) {
  requireUser(req);
  await query(`DELETE FROM payments WHERE status = 'failed'`);
  return new Response("ok");
}
