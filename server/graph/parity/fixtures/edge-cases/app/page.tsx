import Fancy from "~/components/Fancy";
import { supabase } from "../lib";
export default async function Page() {
  const { data } = await supabase.from("orders").select("id, total::int, user:users!fk(email), items(*)").eq("status", "paid").order("created_at");
  await supabase.from("orders").upsert([{ id: 1, total, meta: { a: [1, 2] } }]);
  await supabase.rpc("close_month");
  return <Fancy title="t" />;
}
