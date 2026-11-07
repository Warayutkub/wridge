import { z } from "zod";
import { createPayment } from "@/lib/payments";
import { requireUser } from "@/lib/db";
import { loadOrder } from "../../../lib/payments";
const Body = z.object({ orderId: z.number() });
export async function POST(req: Request) {
  requireUser(req);
  const body = Body.parse(await req.json());
  await loadOrder(body.orderId);
  return Response.json(await createPayment(body.orderId, 100));
}
export async function GET() {
  return Response.json({ ok: true });
}
