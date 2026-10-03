import type { ReactNode } from "react";

/** Pencerenin üst şeridi: sürüklenebilir, başlık solda, denetimler sağda. */
export default function Toolbar({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <header data-tauri-drag-region className="flex h-[52px] shrink-0 items-center gap-3 px-5">
      <h1 data-tauri-drag-region className="min-w-0 truncate text-[15px] font-semibold">
        {title}
      </h1>
      <div data-tauri-drag-region className="h-full flex-1" />
      {children}
    </header>
  );
}
