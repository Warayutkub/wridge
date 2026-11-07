# Flow: Checkout
entry: screen /checkout

## Steps
1. [user] presses "Pay" on `/checkout`
2. [web] POST `/api/checkout` → `createPayment`
3. [⏳ payment] waits for `/api/payments/webhook`
4. [server] `setPaymentStatus`
   - paid → step 5a
   - failed → end
5a. [db] writes `payments, orders`
5b. [email] sends the receipt `sendReceipt`
6. [user] sees "Paid" on `app/orders/[id]/page.tsx`

## States
`payments.status`: pending → paid | failed · paid → refunded (admin only)
`orders.state`: open · closed

## Notes
Webhooks can arrive twice.

## Open questions
- refunds?
