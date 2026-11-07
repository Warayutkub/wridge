import { loadOrder } from "@/lib/payments";
import { CardHeader } from "@/components/ui";
export default async function OrderPage({ params }: { params: { id: string } }) {
  await loadOrder(Number(params.id));
  return <section style={{ background: "#FFFFFF" }}><CardHeader /></section>;
}
