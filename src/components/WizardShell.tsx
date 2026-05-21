import { Lock, ShieldCheck, Sparkles } from "lucide-react";
import { Wordmark } from "@/components/Wordmark";
import { cn } from "@/lib/utils";

export function WizardShell({
  step,
  total,
  title,
  subtitle,
  children,
  footer,
}: {
  step: number;
  total: number;
  title: string;
  subtitle?: string;
  children: React.ReactNode;
  footer?: React.ReactNode;
}) {
  return (
    <main className="flex h-full w-full flex-col bg-canvas text-ink">
      {/* Top strip — wordmark + progress. Acts as a drag region on macOS. */}
      <header
        data-tauri-drag-region
        className="flex h-14 shrink-0 items-center justify-between border-b border-hairline bg-surface px-10"
      >
        <div className="flex items-center gap-3">
          <Wordmark className="h-5 w-auto" />
          <span className="text-[13px] font-medium tracking-[0.2px] text-mute">
            Setup
          </span>
        </div>
        <div className="flex items-center gap-2">
          {Array.from({ length: total }).map((_, i) => (
            <span
              key={i}
              className={cn(
                "h-1.5 rounded-full transition-all",
                i < step ? "w-8 bg-primary" : "w-6 bg-hairline-strong",
              )}
            />
          ))}
          <span className="ml-3 text-[12px] font-medium tabular-nums text-mute">
            {step} / {total}
          </span>
        </div>
      </header>

      {/* Body — fills the viewport. Content is centered inside a roomy column. */}
      <div className="flex min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-[820px] flex-col px-10 py-12">
          {title && (
            <h1 className="text-[36px] font-semibold leading-[1.1] tracking-[-0.01em] text-ink">
              {title}
            </h1>
          )}
          {subtitle && (
            <p className="mt-4 max-w-[640px] text-[16px] leading-[1.6] text-body">
              {subtitle}
            </p>
          )}
          <div
            className={cn(
              "flex flex-1 flex-col",
              title && "mt-10",
            )}
          >
            {children}
          </div>
        </div>
      </div>

      {/* Trust strip — what the user's data actually does. Same line as
          footer nav so primary/secondary buttons stay anchored. */}
      {footer && (
        <footer className="flex shrink-0 items-center justify-between gap-6 border-t border-hairline bg-surface px-10 py-4">
          <div className="hidden items-center gap-4 text-[12px] text-mute md:flex">
            <TrustItem icon={<Lock size={13} />} text="BYOK · 당신 기기에만" />
            <TrustItem icon={<ShieldCheck size={13} />} text="키는 Keychain 저장" />
            <TrustItem icon={<Sparkles size={13} />} text="로컬 vault · 텔레메트리 없음" />
          </div>
          <div className="flex items-center gap-3">{footer}</div>
        </footer>
      )}
    </main>
  );
}

function TrustItem({ icon, text }: { icon: React.ReactNode; text: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className="text-mute">{icon}</span>
      {text}
    </span>
  );
}

export function PrimaryButton({
  children,
  onClick,
  disabled,
  type = "button",
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      className="inline-flex h-11 items-center rounded-md bg-primary px-6 text-[15px] font-semibold leading-none text-on-primary transition-colors hover:bg-primary-pressed disabled:bg-surface-elevated disabled:text-ash"
    >
      {children}
    </button>
  );
}

export function SecondaryButton({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="inline-flex h-11 items-center rounded-md bg-transparent px-4 text-[15px] font-medium leading-none text-on-dark-mute transition-colors hover:text-on-dark disabled:text-stone"
    >
      {children}
    </button>
  );
}

export function TertiaryButton({
  children,
  onClick,
  disabled,
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="inline-flex h-11 items-center rounded-md border border-hairline bg-surface-elevated px-4 text-[15px] font-medium leading-none text-on-dark transition-colors hover:border-hairline-strong disabled:text-ash"
    >
      {children}
    </button>
  );
}

/**
 * Prominent accent button — used for mid-page actions that the user
 * actively needs to click, like "Run connection test". Visually distinct
 * from both the neutral Primary (which lives in the footer) and the
 * low-contrast Tertiary.
 */
export function AccentButton({
  children,
  onClick,
  disabled,
  type = "button",
}: {
  children: React.ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      className="inline-flex h-12 w-full items-center justify-center gap-2 rounded-md border border-accent-blue bg-accent-blue-soft px-5 text-[15px] font-semibold leading-none text-accent-blue transition-colors hover:border-accent-blue hover:bg-accent-blue hover:text-on-dark disabled:border-hairline disabled:bg-surface-elevated disabled:text-ash"
    >
      {children}
    </button>
  );
}

export function TextField({
  label,
  value,
  onChange,
  placeholder,
  type = "text",
  autoComplete,
  monospace,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: "text" | "password";
  autoComplete?: string;
  monospace?: boolean;
}) {
  return (
    <label className="flex flex-col gap-2">
      <span className="text-[12px] font-medium uppercase tracking-[0.6px] text-mute">
        {label}
      </span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        autoComplete={autoComplete}
        className={cn(
          "h-11 w-full rounded-md border border-hairline bg-surface-elevated px-4 text-[15px] text-ink outline-none transition-colors placeholder:text-stone focus:border-hairline-strong focus:bg-surface-card",
          monospace && "font-mono",
        )}
      />
    </label>
  );
}
