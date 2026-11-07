"use client";
import { Button, Card } from "@/components/ui";
import * as UI from "@/components/ui";
export default function CheckoutPage() {
  return <main className="p-4 text-[--color-brand]"><Card title="Pay" /><UI.IconButton icon="x" /><Button label="Pay" variant="primary" /></main>;
}
