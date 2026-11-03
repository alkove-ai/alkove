import type { ReactNode } from 'react'

export const metadata = { title: 'Alkove voice agent' }

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en">
      <body style={{ font: '16px/1.5 system-ui, sans-serif', maxWidth: '40rem', margin: '2rem auto', padding: '0 1rem' }}>{children}</body>
    </html>
  )
}
