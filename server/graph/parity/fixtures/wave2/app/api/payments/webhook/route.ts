import { setPaymentStatus } from "@/lib/payments";
import { requireUser } from "@/lib/db";
export async function POST(req: Request) {
  requireUser(req);
  const { id, paid } = await req.json();
  return Response.json(await setPaymentStatus(id, paid));
}
