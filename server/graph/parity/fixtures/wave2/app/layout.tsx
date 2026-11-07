import "./globals.css";
export default function RootLayout({ children }: { children: React.ReactNode }) {
  return <html><body className="bg-surface">{children}</body></html>;
}
