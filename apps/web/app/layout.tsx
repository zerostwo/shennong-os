import type { Metadata } from "next";
import "./globals.css";
import { QueryProvider } from "@/components/query-provider";
import { NuqsAdapter } from "nuqs/adapters/next/app";

export const metadata: Metadata = {
  title: "Shennong · Biomedical Analysis OS",
  description: "A governed, reproducible operating environment for biomedical data analysis."
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return (
    <html lang="en">
      <body><NuqsAdapter><QueryProvider>{children}</QueryProvider></NuqsAdapter></body>
    </html>
  );
}
