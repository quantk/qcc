// The icon of a project: one of a curated set of lucide icons, optionally in a color
// of the palette, or an emoji. Every place that names a project draws it with this.

import {
  BlocksIcon,
  BookIcon,
  BookOpenIcon,
  BotIcon,
  BrainIcon,
  BriefcaseIcon,
  BrushIcon,
  BugIcon,
  Building2Icon,
  CalendarIcon,
  CameraIcon,
  CarIcon,
  ChartLineIcon,
  CloudIcon,
  CodeIcon,
  CoffeeIcon,
  ContainerIcon,
  CpuIcon,
  DatabaseIcon,
  FileTextIcon,
  FilmIcon,
  FlagIcon,
  FlameIcon,
  FlaskConicalIcon,
  FolderIcon,
  FolderKanbanIcon,
  Gamepad2Icon,
  GitBranchIcon,
  GlobeIcon,
  GraduationCapIcon,
  HardDriveIcon,
  HeadphonesIcon,
  HeartIcon,
  HouseIcon,
  KeyRoundIcon,
  LayersIcon,
  LeafIcon,
  LockIcon,
  type LucideIcon,
  MailIcon,
  MessageSquareIcon,
  MonitorIcon,
  MusicIcon,
  NetworkIcon,
  NotebookPenIcon,
  PackageIcon,
  PaletteIcon,
  PlaneIcon,
  PuzzleIcon,
  RocketIcon,
  ServerIcon,
  ShieldIcon,
  ShoppingCartIcon,
  SmartphoneIcon,
  SparklesIcon,
  StarIcon,
  StoreIcon,
  TerminalIcon,
  TrophyIcon,
  UsersIcon,
  WalletIcon,
  WorkflowIcon,
  WrenchIcon,
  ZapIcon,
} from "lucide-react";
import type { CSSProperties } from "react";

import type { IconColor, ProjectIcon } from "../../api/protocol.gen";
import { cn } from "@/lib/utils";

/** The icon of a project without one, and of one whose icon this app does not know. */
export const DEFAULT_PROJECT_ICON = "folder-kanban";

/**
 * The icons a project can take, by lucide's kebab-case name, in the order the
 * picker shows them: work first, then the rest of life. The daemon keeps any
 * well-formed name, so a name dropped from here falls back to the default.
 */
export const PROJECT_ICONS: Readonly<Record<string, LucideIcon>> = {
  "folder-kanban": FolderKanbanIcon,
  folder: FolderIcon,
  code: CodeIcon,
  terminal: TerminalIcon,
  bot: BotIcon,
  brain: BrainIcon,
  sparkles: SparklesIcon,
  globe: GlobeIcon,
  server: ServerIcon,
  database: DatabaseIcon,
  cloud: CloudIcon,
  container: ContainerIcon,
  network: NetworkIcon,
  cpu: CpuIcon,
  "hard-drive": HardDriveIcon,
  smartphone: SmartphoneIcon,
  monitor: MonitorIcon,
  package: PackageIcon,
  blocks: BlocksIcon,
  puzzle: PuzzleIcon,
  layers: LayersIcon,
  workflow: WorkflowIcon,
  "git-branch": GitBranchIcon,
  bug: BugIcon,
  rocket: RocketIcon,
  zap: ZapIcon,
  flame: FlameIcon,
  wrench: WrenchIcon,
  shield: ShieldIcon,
  lock: LockIcon,
  "key-round": KeyRoundIcon,
  book: BookIcon,
  "book-open": BookOpenIcon,
  "notebook-pen": NotebookPenIcon,
  "file-text": FileTextIcon,
  "graduation-cap": GraduationCapIcon,
  "flask-conical": FlaskConicalIcon,
  "chart-line": ChartLineIcon,
  briefcase: BriefcaseIcon,
  wallet: WalletIcon,
  store: StoreIcon,
  "shopping-cart": ShoppingCartIcon,
  house: HouseIcon,
  "building-2": Building2Icon,
  users: UsersIcon,
  mail: MailIcon,
  "message-square": MessageSquareIcon,
  calendar: CalendarIcon,
  "gamepad-2": Gamepad2Icon,
  music: MusicIcon,
  film: FilmIcon,
  camera: CameraIcon,
  palette: PaletteIcon,
  brush: BrushIcon,
  headphones: HeadphonesIcon,
  coffee: CoffeeIcon,
  leaf: LeafIcon,
  heart: HeartIcon,
  star: StarIcon,
  trophy: TrophyIcon,
  flag: FlagIcon,
  plane: PlaneIcon,
  car: CarIcon,
};

/** The palette, in the order the picker shows it. The shades follow the theme. */
export const ICON_COLORS: readonly { value: IconColor; label: string }[] = [
  { value: "brand", label: "Amber" },
  { value: "red", label: "Red" },
  { value: "orange", label: "Orange" },
  { value: "yellow", label: "Yellow" },
  { value: "green", label: "Green" },
  { value: "teal", label: "Teal" },
  { value: "blue", label: "Blue" },
  { value: "violet", label: "Violet" },
  { value: "pink", label: "Pink" },
  { value: "gray", label: "Gray" },
];

/** The CSS color of a palette color: a variable of `styles.css`, so it follows the theme. */
export function iconColorValue(color: IconColor): string {
  return color === "gray" ? "var(--text-muted)" : `var(--${color})`;
}

/** The lucide icon a name stands for; the default for a name this app does not know. */
export function lucideIconOf(name: string): LucideIcon {
  return (Object.hasOwn(PROJECT_ICONS, name) ? PROJECT_ICONS[name] : undefined) ?? FolderKanbanIcon;
}

/** What a project's icon is, in words, for titles and screen readers. */
export function projectIconLabel(icon: ProjectIcon | null | undefined): string {
  if (!icon) return "Default icon";
  if (icon.kind === "emoji") return icon.emoji;
  const color = icon.color ? ICON_COLORS.find((candidate) => candidate.value === icon.color)?.label : undefined;
  return color ? `${icon.name}, ${color.toLowerCase()}` : icon.name;
}

/**
 * A project's icon, `size` pixels square. Without an icon it is the default one in
 * the color of the text around it; `className` sets that color.
 */
export function ProjectIconView({
  icon,
  size = 14,
  className,
}: {
  icon: ProjectIcon | null | undefined;
  size?: number;
  className?: string;
}) {
  if (icon?.kind === "emoji") {
    // A box of the icon's size keeps rows aligned; a wide sequence is clipped, not spilled.
    const style: CSSProperties = { width: size, height: size, fontSize: Math.round(size * 0.9), lineHeight: 1 };
    return (
      <span
        aria-hidden
        className={cn("inline-flex flex-none items-center justify-center overflow-hidden", className)}
        style={style}
      >
        {icon.emoji}
      </span>
    );
  }
  const Icon = lucideIconOf(icon?.name ?? DEFAULT_PROJECT_ICON);
  const color = icon?.color ? { color: iconColorValue(icon.color) } : undefined;
  return <Icon aria-hidden className={cn("flex-none", className)} style={{ width: size, height: size, ...color }} />;
}
