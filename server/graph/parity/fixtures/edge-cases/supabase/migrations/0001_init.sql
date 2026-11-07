create table public.profiles (id uuid primary key references auth.users(id), handle citext unique);
alter table profiles enable row level security;
create policy "own" on profiles for select to authenticated using (id = auth.uid()) with check (true);
create or replace function close_month() returns void language plpgsql as $fn$ begin update orders set total = 0; end $fn$;
create view v_users as select * from profiles join orders on true;
