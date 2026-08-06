import type { Metadata, Viewport } from "next";
import "./globals.css";
import { Toaster } from "@/components/Toaster";

export const metadata: Metadata = {
  title: "APAS - Claude Code Remote",
  description: "Web interface for Claude Code",
  manifest: "/manifest.json",
  appleWebApp: {
    capable: true,
    statusBarStyle: "default",
    title: "APAS",
  },
  formatDetection: {
    telephone: false,
  },
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  maximumScale: 1,
  userScalable: false,
  themeColor: "#7c3aed",
  viewportFit: "cover", // Required for iOS safe area handling
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="antialiased">
        {/* Applied before first paint. Without this the page renders in the
            default theme and then snaps to the chosen one on hydration, which
            is very visible on a dark theme. Inline and synchronous on purpose;
            it must run before the body is painted. Mirrors `applyTheme` in
            lib/theme.ts — keep the two in step. */}
        <script
          dangerouslySetInnerHTML={{
            __html: `(function(){try{
var t=localStorage.getItem('apas_theme')||'system';
var d=window.matchMedia&&window.matchMedia('(prefers-color-scheme: dark)').matches;
var isDark=t==='dark'||t==='solarized-dark'||(t==='system'&&d);
var r=document.documentElement;
r.classList.toggle('dark',isDark);
if(t==='system'){r.removeAttribute('data-theme')}else{r.setAttribute('data-theme',t)}
}catch(e){}})()`,
          }}
        />
        {children}
        <Toaster />
      </body>
    </html>
  );
}
