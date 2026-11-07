import Stripe from "stripe";
import { query } from "./db";
const stripe = new Stripe("sk_test");
export async function createPayment(orderId: number, amount: number) {
  await query(`INSERT INTO payments (order_id, amount, status) VALUES (${orderId}, ${amount}, 'pending')`);
  const status = 'pending';
  return stripe.paymentIntents.create({ amount, currency: "thb" });
}
export async function setPaymentStatus(id: number, paid: boolean) {
  const row = { status: paid ? "paid" : "failed" };
  await query(`UPDATE payments SET status = 'paid' WHERE id = ${id}`);
  await query(`UPDATE orders SET state = 'closed' WHERE id = ${id}`);
  return row;
}
export function loadOrder(id: number) {
  return query(`SELECT id, total, state FROM orders WHERE id = ${id}`);
}
