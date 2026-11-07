export default function Card({ title, ...rest }: { title: string }) { return <div {...rest}>{title}</div>; }
