import { Component, type ErrorInfo, type ReactNode } from "react";
import { CircleAlert } from "lucide-react";
import { api, logClient } from "../api";
import { toast } from "../lib/feedback";
import { Button } from "./ui/button";

type Props = {
  children: ReactNode;
  /** Değişince (örn. başka sayfaya geçince) hata unutulur, içerik yeniden çizilir. */
  resetKey?: unknown;
};
type State = { error: Error | null; resetKey: unknown };

/**
 * Çizimde atılan hatayı yakalar: tek bir bölüm bozulunca bütün pencere boş kalmaz, hata
 * günlüğe yazılır ve "Yeniden dene" sunulur.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, resetKey: this.props.resetKey };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    return props.resetKey !== state.resetKey ? { error: null, resetKey: props.resetKey } : null;
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    logClient("error", `çizim hatası: ${error.stack ?? error.message}\n${info.componentStack ?? ""}`);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div role="alert" className="grid flex-1 place-items-center p-8">
        <div className="flex max-w-sm flex-col items-center gap-3 text-center">
          <CircleAlert className="size-8 text-destructive" />
          <div className="text-sm font-medium">Bu bölüm açılamadı</div>
          <p className="text-xs text-muted-foreground">
            Hata günlüğe yazıldı. Yeniden denemek çoğu zaman yeter; sürerse Ayarlar → Sorun giderme'den tanılama
            bilgisini kopyalayıp paylaş.
          </p>
          <pre className="max-h-24 w-full overflow-auto rounded-md bg-muted/60 px-3 py-2 text-left font-mono text-[11px] whitespace-pre-wrap text-muted-foreground">
            {error.message}
          </pre>
          <div className="flex gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                api
                  .diagnostics()
                  .then((text) => navigator.clipboard.writeText(text))
                  .then(() => toast("Tanılama bilgisi kopyalandı", { tone: "success" }))
                  .catch(() => {})
              }
            >
              Tanılamayı kopyala
            </Button>
            <Button size="sm" onClick={() => this.setState({ error: null })}>
              Yeniden dene
            </Button>
          </div>
        </div>
      </div>
    );
  }
}

/** Yakalanmayan istisnalar ve reddedilen sözler de günlüğe düşsün. */
export function installGlobalErrorLogging() {
  window.addEventListener("error", (e) => {
    logClient("error", `yakalanmayan hata: ${e.error?.stack ?? e.message}`);
  });
  window.addEventListener("unhandledrejection", (e) => {
    const r = e.reason;
    logClient("error", `yakalanmayan söz: ${r instanceof Error ? (r.stack ?? r.message) : String(r)}`);
  });
}
