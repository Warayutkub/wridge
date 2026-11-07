create table public.orders (id bigint primary key, total numeric not null, state text default 'open', created_at timestamptz);
create table public.payments (id bigint primary key, order_id bigint references public.orders(id), amount numeric, status text not null default 'pending');
create index payments_order on public.payments (order_id);
alter table public.payments enable row level security;
create policy "own payments" on public.payments for select using (auth.uid() is not null);
create view public.paid_orders as select o.id from public.orders o join public.payments p on p.order_id = o.id where p.status = 'paid';
