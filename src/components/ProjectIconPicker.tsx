import { useMemo, useState } from "react";
import {
  Folder,
  Bot,
  Brain,
  Briefcase,
  Bug,
  Calendar,
  Camera,
  ChartBar,
  Cloud,
  Code,
  Coffee,
  Compass,
  Database,
  FileText,
  Film,
  Flag,
  FlaskConical,
  Gamepad2,
  GitBranch,
  Globe,
  GraduationCap,
  Hammer,
  Headphones,
  Heart,
  Home,
  Image,
  Key,
  Lightbulb,
  Map,
  Megaphone,
  MessageCircle,
  Mic,
  Music,
  Package,
  Palette,
  PenTool,
  Pencil,
  Phone,
  Plane,
  Puzzle,
  Rocket,
  Scale,
  Server,
  Settings,
  Shield,
  ShoppingBag,
  Smartphone,
  Sparkles,
  Star,
  Sun,
  Target,
  Terminal,
  Trophy,
  Truck,
  Tv,
  Umbrella,
  User,
  Users,
  Video,
  Wand,
  Wifi,
  Wrench,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

/** Curated subset of lucide icons. Keys are the literal lucide names so
 *  the value stored in config (`project_icons`) round-trips exactly.
 *  Keep this list small (<100) — a giant grid is harder to scan than
 *  text-named filters. */
export const PROJECT_ICONS: Record<string, LucideIcon> = {
  Folder,
  // Dev / engineering
  Code,
  Terminal,
  GitBranch,
  Bug,
  Database,
  Server,
  Wrench,
  Hammer,
  Package,
  Cloud,
  Wifi,
  Settings,
  Key,
  Shield,
  // AI / ideas
  Bot,
  Brain,
  Sparkles,
  Lightbulb,
  Zap,
  Wand,
  Puzzle,
  // Knowledge / writing
  FileText,
  Pencil,
  PenTool,
  Palette,
  GraduationCap,
  FlaskConical,
  // Product / business
  Briefcase,
  Target,
  Rocket,
  Flag,
  Trophy,
  ChartBar,
  Megaphone,
  ShoppingBag,
  Scale,
  // Communication / people
  MessageCircle,
  Users,
  User,
  Phone,
  Mic,
  Headphones,
  Camera,
  Video,
  // Daily / life
  Calendar,
  Home,
  Heart,
  Star,
  Coffee,
  Sun,
  Umbrella,
  Compass,
  Map,
  Globe,
  Plane,
  Truck,
  // Media / hobby
  Music,
  Film,
  Tv,
  Image,
  Gamepad2,
  Smartphone,
};

const ICON_NAMES = Object.keys(PROJECT_ICONS);

/** Resolve an icon name back to a component. Returns Folder when the
 *  name is unknown — config files survive lucide rename/delete. */
export function projectIconOf(name: string | null | undefined): LucideIcon {
  if (!name) return Folder;
  return PROJECT_ICONS[name] ?? Folder;
}

export function ProjectIconPicker({
  value,
  onSelect,
  onClear,
  onClose,
}: {
  value: string | null;
  onSelect: (iconName: string) => void;
  onClear: () => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return ICON_NAMES;
    return ICON_NAMES.filter((n) => n.toLowerCase().includes(q));
  }, [query]);

  return (
    <div className="flex flex-col gap-3">
      <input
        autoFocus
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
        }}
        placeholder="아이콘 검색 (예: bot, code, brain)"
        className="h-9 w-full rounded-md border border-hairline bg-surface-elevated px-3 text-[13px] text-ink outline-none placeholder:text-stone focus:border-hairline-strong"
      />
      <div className="grid max-h-[320px] grid-cols-8 gap-1 overflow-y-auto rounded-md border border-hairline bg-surface-elevated p-2">
        {filtered.length === 0 && (
          <div className="col-span-8 py-6 text-center text-caption-sm text-mute">
            매칭되는 아이콘이 없어요.
          </div>
        )}
        {filtered.map((name) => {
          const Icon = PROJECT_ICONS[name];
          const active = value === name;
          return (
            <button
              key={name}
              type="button"
              onClick={() => onSelect(name)}
              title={name}
              className={cn(
                "grid h-9 w-9 place-items-center rounded-md transition-colors",
                active
                  ? "bg-accent-blue-soft text-accent-blue ring-1 ring-accent-blue"
                  : "text-body hover:bg-surface-card hover:text-on-dark",
              )}
            >
              <Icon size={16} />
            </button>
          );
        })}
      </div>
      <div className="flex items-center justify-between text-caption-sm text-mute">
        <span>{filtered.length} / {ICON_NAMES.length} 개</span>
        <button
          type="button"
          onClick={onClear}
          className="rounded-sm px-2 py-1 text-mute hover:text-ink"
        >
          기본으로 되돌리기
        </button>
      </div>
    </div>
  );
}
