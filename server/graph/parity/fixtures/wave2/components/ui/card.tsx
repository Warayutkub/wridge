import { Button } from "./button";
export default function Card({ title }: { title: string }) {
  return <div className="card shadow-card" style={{ color: "#1e40af" }}><h2>{title}</h2><Button label="ok" /></div>;
}
export function CardHeader() {
  return <header className="card-header">x</header>;
}
