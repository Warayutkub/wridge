export function Button({ label, variant }: { label: string; variant?: "primary" | "danger" }) {
  return <button className={`btn bg-primary text-sm ${variant === "danger" ? "btn-d" : "btn-p"}`}>{label}</button>;
}
export const IconButton = ({ icon }: { icon: string }) => <Button label={icon} />;
