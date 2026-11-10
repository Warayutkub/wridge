// Brewly: the made-up demo project the website screenshots come from. node tools/site/make-brewly.mjs C:/demo/brewly
// Brewly: a made-up coffee-ordering app used only for wridge's website screenshots.
import { mkdirSync, writeFileSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
const ROOT = process.argv[2];
rmSync(ROOT, { recursive: true, force: true });
const w = (p, s) => { const f = join(ROOT, p); mkdirSync(dirname(f), { recursive: true }); writeFileSync(f, s.replace(/^\n/, "")); };

// ---------------- brain
const note = (folder, name, tags, body) => w(`brain/${folder ? folder + "/" : ""}${name}.md`, `---\ntags: [${tags.join(", ")}]\n---\n# ${name}\n\n${body}\n`);
note("", "Brewly", ["product"], `A coffee-ordering app for small cafés: order ahead, pay in the app, pick up at the counter.\n\nStart here: [[Order flow]] · [[Menu]] · [[Payments]] · [[Design system]] · [[Launch plan]]`);
note("product", "Launch plan", ["product"], `Soft launch in two cafés, then [[Loyalty points]]. Blockers: [[Refunds]], [[Opening hours]].\nOwner: product. See [[Brewly]].`);
note("product", "Personas", ["product"], `Morning commuter (orders on the way, picks up in 5 min) and the café barista (sees the queue). Drives [[Order flow]] and [[Barista queue]].`);
note("product", "No tipping in v1", ["product", "decision"], `Agreed 2026-10-12: no tips in v1, keep checkout to one screen. Revisit after [[Launch plan]]. Affects [[Checkout]].`);
note("menu", "Menu", ["menu"], `Items, sizes and add-ons per café. Stored in \`menu_items\`. Built from [[Menu sync]]; shown with [[MenuCard]].`);
note("menu", "Menu sync", ["menu"], `Cafés edit the menu in a sheet; a job imports it nightly. Out-of-stock flips \`available\`. See [[Menu]].`);
note("menu", "Add-ons", ["menu"], `Oat milk, extra shot, syrups. Priced per café. Part of [[Menu]], summed in [[Checkout]].`);
note("ordering", "Order flow", ["ordering"], `Browse [[Menu]] → cart → [[Checkout]] → [[Barista queue]] → ready → picked up. States in [[Order states]].`);
note("ordering", "Order states", ["ordering"], `placed → paid → brewing → ready → picked_up, or cancelled / refunded. Every change is written to \`order_events\`. See [[Order flow]].`);
note("ordering", "Barista queue", ["ordering"], `The café tablet: paid orders in time order, tap to start, tap when ready. Push to the customer when ready. Part of [[Order flow]].`);
note("ordering", "Opening hours", ["ordering"], `No orders 10 minutes before closing. Per café, in \`cafes\`. Blocks [[Checkout]] outside hours.`);
note("payments", "Payments", ["payments"], `Card and wallet through the payment provider. One charge per order, captured when the barista starts. See [[Checkout]], [[Refunds]].`);
note("payments", "Checkout", ["payments"], `One screen: items, add-ons, pickup time, pay. Totals include [[Add-ons]]. [[No tipping in v1]].`);
note("payments", "Refunds", ["payments"], `Barista can refund before \`ready\`. After that only the café owner. Writes \`payments.refunded_at\`. Part of [[Payments]].`);
note("payments", "Loyalty points", ["payments"], `1 point per $1, 50 points = free drink. Table \`loyalty_points\`. After [[Launch plan]].`);
note("design", "Design system", ["design"], `Warm cream, espresso brown, one accent. Components: [[MenuCard]], buttons, price tags. Figma file "Brewly app".`);
note("design", "MenuCard", ["design"], `Photo, name, price, add button. Sold-out state greys it out. Used by [[Menu]].`);
note("design", "Empty states", ["design"], `Empty cart, café closed ([[Opening hours]]), no orders yet. Part of [[Design system]].`);

// ---------------- data (migrations)
w("db/migrations/001_cafes_menu.sql", `
create table cafes (
  id uuid primary key,
  name text not null,
  opens_at time not null,
  closes_at time not null
);
create table menu_items (
  id uuid primary key,
  cafe_id uuid not null references cafes(id),
  name text not null,
  size text,
  price_cents integer not null,
  available boolean not null default true
);
`);
w("db/migrations/002_customers_orders.sql", `
create table customers (
  id uuid primary key,
  email text not null unique,
  name text,
  created_at timestamptz not null default now()
);
create table orders (
  id uuid primary key,
  cafe_id uuid not null references cafes(id),
  customer_id uuid not null references customers(id),
  status text not null default 'placed',
  pickup_at timestamptz,
  total_cents integer not null
);
create table order_items (
  id uuid primary key,
  order_id uuid not null references orders(id),
  menu_item_id uuid not null references menu_items(id),
  quantity integer not null default 1,
  addons jsonb
);
create table order_events (
  id bigserial primary key,
  order_id uuid not null references orders(id),
  status text not null,
  at timestamptz not null default now()
);
`);
w("db/migrations/003_payments_loyalty.sql", `
create table payments (
  id uuid primary key,
  order_id uuid not null references orders(id),
  provider_ref text not null,
  amount_cents integer not null,
  captured_at timestamptz,
  refunded_at timestamptz
);
create table loyalty_points (
  customer_id uuid not null references customers(id),
  points integer not null default 0,
  updated_at timestamptz not null default now()
);
`);

// ---------------- code (Next.js app router)
w("package.json", JSON.stringify({ name: "brewly", private: true, scripts: { dev: "next dev" }, dependencies: { next: "15.0.0", react: "19.0.0", "react-dom": "19.0.0", postgres: "3.4.4" }, devDependencies: { typescript: "5.6.3" } }, null, 2) + "\n");
w("tsconfig.json", JSON.stringify({ compilerOptions: { strict: true, jsx: "preserve", baseUrl: ".", paths: { "@/*": ["./*"] } } }, null, 2) + "\n");
w("lib/db.ts", `import postgres from "postgres";\nexport const sql = postgres(process.env.DATABASE_URL!);\n`);
w("lib/menu.ts", `import { sql } from "./db";
export type MenuItem = { id: string; name: string; size: string | null; price_cents: number; available: boolean };
export async function listMenu(cafeId: string): Promise<MenuItem[]> {
  return sql\`select id, name, size, price_cents, available from menu_items where cafe_id = \${cafeId} order by name\`;
}
export async function setAvailable(id: string, available: boolean) {
  await sql\`update menu_items set available = \${available} where id = \${id}\`;
}
`);
w("lib/cafes.ts", `import { sql } from "./db";
export async function isOpen(cafeId: string, at = new Date()) {
  const [c] = await sql\`select opens_at, closes_at from cafes where id = \${cafeId}\`;
  return Boolean(c) && at.toTimeString() < c.closes_at;
}
`);
w("lib/orders.ts", `import { sql } from "./db";
import { isOpen } from "./cafes";
import { capture } from "./payments";
export type OrderStatus = "placed" | "paid" | "brewing" | "ready" | "picked_up" | "cancelled" | "refunded";
export async function placeOrder(cafeId: string, customerId: string, items: { menuItemId: string; quantity: number }[], totalCents: number) {
  if (!(await isOpen(cafeId))) throw new Error("closed");
  const [o] = await sql\`insert into orders (id, cafe_id, customer_id, total_cents) values (gen_random_uuid(), \${cafeId}, \${customerId}, \${totalCents}) returning id\`;
  for (const i of items) await sql\`insert into order_items (id, order_id, menu_item_id, quantity) values (gen_random_uuid(), \${o.id}, \${i.menuItemId}, \${i.quantity})\`;
  await logEvent(o.id, "placed");
  return o.id as string;
}
export async function setStatus(orderId: string, status: OrderStatus) {
  await sql\`update orders set status = \${status} where id = \${orderId}\`;
  await logEvent(orderId, status);
  if (status === "brewing") await capture(orderId);
}
export async function queue(cafeId: string) {
  return sql\`select id, status, pickup_at from orders where cafe_id = \${cafeId} and status in ('paid', 'brewing') order by pickup_at\`;
}
async function logEvent(orderId: string, status: OrderStatus) {
  await sql\`insert into order_events (order_id, status) values (\${orderId}, \${status})\`;
}
`);
w("lib/payments.ts", `import { sql } from "./db";
import { addPoints } from "./loyalty";
export async function capture(orderId: string) {
  const [p] = await sql\`update payments set captured_at = now() where order_id = \${orderId} returning amount_cents\`;
  const [o] = await sql\`select customer_id from orders where id = \${orderId}\`;
  await addPoints(o.customer_id, Math.floor(p.amount_cents / 100));
}
export async function refund(orderId: string) {
  await sql\`update payments set refunded_at = now() where order_id = \${orderId}\`;
}
`);
w("lib/loyalty.ts", `import { sql } from "./db";
export async function addPoints(customerId: string, points: number) {
  await sql\`update loyalty_points set points = points + \${points}, updated_at = now() where customer_id = \${customerId}\`;
}
export async function balance(customerId: string) {
  const [r] = await sql\`select points from loyalty_points where customer_id = \${customerId}\`;
  return r?.points ?? 0;
}
`);
w("components/Button.tsx", `export function Button({ children, onClick }: { children: React.ReactNode; onClick?: () => void }) {\n  return <button className="btn" onClick={onClick}>{children}</button>;\n}\n`);
w("components/PriceTag.tsx", `export function PriceTag({ cents }: { cents: number }) {\n  return <span className="price">\${(cents / 100).toFixed(2)}</span>;\n}\n`);
w("components/MenuCard.tsx", `import { Button } from "./Button";\nimport { PriceTag } from "./PriceTag";\nimport type { MenuItem } from "@/lib/menu";\nexport function MenuCard({ item, onAdd }: { item: MenuItem; onAdd: () => void }) {\n  return <div className={item.available ? "card" : "card sold-out"}><h3>{item.name}</h3><PriceTag cents={item.price_cents} /><Button onClick={onAdd}>Add</Button></div>;\n}\n`);
w("components/CartItem.tsx", `import { PriceTag } from "./PriceTag";\nexport function CartItem({ name, cents, quantity }: { name: string; cents: number; quantity: number }) {\n  return <li>{quantity} × {name} <PriceTag cents={cents * quantity} /></li>;\n}\n`);
w("components/EmptyState.tsx", `export function EmptyState({ title }: { title: string }) {\n  return <div className="empty">{title}</div>;\n}\n`);
w("components/OrderTicket.tsx", `import { Button } from "./Button";\nexport function OrderTicket({ id, status, onNext }: { id: string; status: string; onNext: () => void }) {\n  return <div className="ticket">#{id.slice(0, 4)} · {status}<Button onClick={onNext}>Next</Button></div>;\n}\n`);
w("app/page.tsx", `import Link from "next/link";\nexport default function Home() {\n  return <main><h1>Brewly</h1><Link href="/menu">Order coffee</Link></main>;\n}\n`);
w("app/menu/page.tsx", `import { listMenu } from "@/lib/menu";\nimport { MenuCard } from "@/components/MenuCard";\nimport { EmptyState } from "@/components/EmptyState";\nexport default async function MenuPage({ searchParams }: { searchParams: { cafe: string } }) {\n  const items = await listMenu(searchParams.cafe);\n  if (!items.length) return <EmptyState title="Nothing on the menu yet" />;\n  return <section>{items.map((i) => <MenuCard key={i.id} item={i} onAdd={() => {}} />)}</section>;\n}\n`);
w("app/cart/page.tsx", `import { CartItem } from "@/components/CartItem";\nimport { EmptyState } from "@/components/EmptyState";\nimport { Button } from "@/components/Button";\nexport default function CartPage() {\n  const items: { name: string; cents: number; quantity: number }[] = [];\n  if (!items.length) return <EmptyState title="Your cart is empty" />;\n  return <ul>{items.map((i) => <CartItem key={i.name} {...i} />)}<Button>Checkout</Button></ul>;\n}\n`);
w("app/checkout/page.tsx", `import { Button } from "@/components/Button";\nimport { PriceTag } from "@/components/PriceTag";\nexport default function CheckoutPage() {\n  return <form action="/api/orders" method="post"><PriceTag cents={0} /><Button>Pay</Button></form>;\n}\n`);
w("app/orders/[id]/page.tsx", `import { sql } from "@/lib/db";\nexport default async function OrderPage({ params }: { params: { id: string } }) {\n  const [o] = await sql\`select status, pickup_at from orders where id = \${params.id}\`;\n  return <p>Your order is {o.status}</p>;\n}\n`);
w("app/barista/page.tsx", `import { queue } from "@/lib/orders";\nimport { OrderTicket } from "@/components/OrderTicket";\nexport default async function BaristaPage({ searchParams }: { searchParams: { cafe: string } }) {\n  const orders = await queue(searchParams.cafe);\n  return <section>{orders.map((o: any) => <OrderTicket key={o.id} id={o.id} status={o.status} onNext={() => {}} />)}</section>;\n}\n`);
w("app/rewards/page.tsx", `import { balance } from "@/lib/loyalty";\nexport default async function RewardsPage() {\n  const points = await balance("me");\n  return <p>{points} points</p>;\n}\n`);
w("app/api/orders/route.ts", `import { placeOrder } from "@/lib/orders";\nexport async function POST(req: Request) {\n  const b = await req.json();\n  const id = await placeOrder(b.cafeId, b.customerId, b.items, b.totalCents);\n  return Response.json({ id });\n}\n`);
w("app/api/orders/[id]/status/route.ts", `import { setStatus } from "@/lib/orders";\nexport async function POST(req: Request, { params }: { params: { id: string } }) {\n  await setStatus(params.id, (await req.json()).status);\n  return Response.json({ ok: true });\n}\n`);
w("app/api/refunds/route.ts", `import { refund } from "@/lib/payments";\nexport async function POST(req: Request) {\n  await refund((await req.json()).orderId);\n  return Response.json({ ok: true });\n}\n`);
w("app/api/menu/sync/route.ts", `import { setAvailable } from "@/lib/menu";\nexport async function POST(req: Request) {\n  for (const r of await req.json()) await setAvailable(r.id, r.available);\n  return Response.json({ ok: true });\n}\n`);
w("README.md", `# Brewly\n\nA made-up coffee-ordering app. It exists only so wridge's website can show real screenshots of the Brain, Code graph, Data graph, Tasks and Reports without anyone's real project.\n`);
// ---------------- tasks board and a weekly report (the Tasks and Reports screenshots)
{
  const at = (d, h = 10) => `2026-11-${String(d).padStart(2, "0")}T${String(h).padStart(2, "0")}:00:00+07:00`;
  let n = 0;
  const T = (o) => ({ id: `T-${++n}`, title: "", status: "todo", description: "", labels: [], subtasks: [], links: [], due: "", who: "", from: "", by: "you", created: at(8), updated: at(9), activity: [{ at: at(8), who: o.by ?? "you", text: o.from ? "suggested this" : "added this" }], ...o });
  const tasks = [
    T({ title: "Pickup time picker", description: "Let people choose when to collect their order.", labels: ["Feature"], who: "you", subtasks: [{ text: "Slots every 10 minutes", done: true }, { text: "Hide slots when the café is full", done: false }, { text: "Show the time on the receipt", done: false }], due: "2026-11-14" }),
    T({ title: "Dark mode for the menu", labels: ["UI"], who: "AI", by: "AI", subtasks: [{ text: "Colors from the design tokens", done: false }] }),
    T({ title: "Back up orders every night", description: "cron 03:00, keep 14 days", labels: ["Ops"], who: "you", due: "2026-11-09" }),
    T({ title: "Cart: remember items after a refresh", status: "doing", labels: ["Feature"], who: "AI", by: "AI", subtasks: [{ text: "Save to local storage", done: true }, { text: "Merge with the server cart on sign-in", done: true }, { text: "Test with two tabs open", done: false }], links: ["app/cart/page.tsx"] }),
    T({ title: "Speed up the menu query", status: "doing", labels: ["Perf"], who: "you", subtasks: [{ text: "Add an index on category", done: true }, { text: "Cache for 60 s", done: false }] }),
    T({ title: "Typo on the receipt", status: "review", labels: ["Copy"], who: "AI", by: "AI", from: "AI check · 10 Nov", change: { where: "receipt.html:42", old: "Thank you for you order", new: "Thank you for your order" } }),
    T({ title: "Payment call has no timeout", status: "review", labels: ["Bug"], who: "AI", by: "AI", from: "AI check · 10 Nov", change: { where: "lib/payments.ts:18", old: "await fetch(url)", new: "await fetch(url, { signal: AbortSignal.timeout(10_000) })" } }),
    T({ title: "Save button has no loading state", status: "review", labels: ["UI"], who: "AI", by: "AI", from: "AI check · 10 Nov", description: "Figma · Checkout · Form" }),
    T({ title: "Sign in with LINE", status: "done", labels: ["Feature"], who: "you", subtasks: [{ text: "Login", done: true }, { text: "Link to an existing account", done: true }] }),
    T({ title: "Menu photos load slowly", status: "done", labels: ["Perf"], who: "AI", by: "AI" }),
  ];
  w("docs/tasks/board.json", JSON.stringify({ version: 1, next: n + 1, tasks }, null, 2));
  w("docs/reports/week-45.json", JSON.stringify({
    title: "Brewly · Week 45",
    rows: [
      { type: "header", eyebrow: "BREWLY · WEEKLY REPORT · 10 NOV 2026", title: "Week 45: checkout is faster", subtitle: "What shipped, how orders moved, and what the AI found on its last check.", status: { text: "On track", kind: "ok" }, meta: { release: "2.4.0", team: "3 people + AI" } },
      { type: "stats", items: [{ label: "Orders", value: "1,284", delta: "+18%", kind: "ok" }, { label: "Checkout time", value: "41 s", delta: "−12 s", kind: "ok" }, { label: "Tasks done", value: "9", note: "of 12 planned" }, { label: "Open bugs", value: "2", delta: "−3", kind: "ok" }] },
      [
        { type: "chart", title: "Orders per day", kind: "line", span: 2, x: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"], series: [{ name: "This week", values: [142, 156, 171, 165, 210, 248, 192] }, { name: "Last week", values: [120, 131, 140, 138, 175, 202, 181] }], marks: [{ x: "Wed", text: "2.4.0" }] },
        { type: "chart", title: "Where orders come from", kind: "donut", items: [{ label: "App", value: 61 }, { label: "LINE", value: 27 }, { label: "Walk-in", value: 12 }], center: { value: "1,284", label: "orders" } },
      ],
      [
        { type: "checklist", title: "Shipped this week", items: [{ text: "Sign in with LINE", done: true, note: "2.4.0" }, { text: "Menu photos load 3× faster", done: true }, { text: "Cart remembers items", done: false, note: "in review" }] },
        { type: "tasks", title: "Waiting for your yes", eyebrow: "AI check · 10 Nov", filter: { from: "AI check" }, view: "list" },
      ],
    ],
  }, null, 2));
}
console.log("brewly written to", ROOT);
