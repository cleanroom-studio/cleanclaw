// Shared UI primitives. Tailwind-only, no extra deps.

export const buttonClasses =
  'inline-flex items-center justify-center gap-2 rounded-md border border-transparent bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:opacity-90 disabled:pointer-events-none disabled:opacity-50';

export const inputClasses =
  'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring';

export const cardClasses =
  'rounded-lg border border-border bg-card p-6 shadow-sm';

export const labelClasses =
  'text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70';

export const badgeClasses =
  'inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2';

export const tableClasses =
  'w-full caption-bottom text-sm';

export const theadClasses =
  'border-b';

export const thClasses =
  'h-10 px-2 text-left align-middle font-medium text-muted-foreground [&:has([role=checkbox])]:pr-0 [&>[role=checkbox]]:translate-y-[2px]';

export const tdClasses =
  'p-2 align-middle [&:has([role=checkbox])]:pr-0 [&>[role=checkbox]]:translate-y-[2px]';

export const dialogOverlayClasses =
  'fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0';

export const dialogContentClasses =
  'fixed left-[50%] top-[50%] z-50 grid w-full max-w-lg translate-x-[-50%] translate-y-[-50%] gap-4 border border-border bg-background p-6 shadow-lg sm:rounded-lg';

export const skeletonClasses =
  'animate-pulse rounded-md bg-muted';

export const separatorClasses =
  'shrink-0 bg-border h-[1px] w-full';

export const textareaClasses =
  'flex min-h-[60px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring';

export const switchClasses =
  'peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input';

export const tabsListClasses =
  'inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground';

export const tabsTriggerClasses =
  'inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-all focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow';

export const selectTriggerClasses =
  'flex h-9 w-full items-center justify-between whitespace-nowrap rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm ring-offset-background placeholder:text-muted-foreground focus:outline-none focus:ring-1 focus:ring-ring disabled:cursor-not-allowed disabled:opacity-50 [&>span]:line-clamp-1';

export const selectContentClasses =
  'relative z-50 max-h-96 min-w-[8rem] overflow-hidden rounded-md border border-border bg-popover text-popover-foreground shadow-md';

export const selectItemClasses =
  'relative flex w-full cursor-default select-none items-center rounded-sm py-1.5 pl-2 pr-8 text-sm outline-none focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50';

export const scrollAreaClasses =
  'relative overflow-y-auto overflow-x-hidden';

export const avatarClasses =
  'relative flex h-9 w-9 shrink-0 overflow-hidden rounded-full';

export const sheetOverlayClasses =
  'fixed inset-0 z-50 bg-black/80';

export const sheetContentClasses =
  'fixed z-50 gap-4 bg-background p-6 shadow-lg border border-border';

export const sheetContentRight =
  'inset-y-0 right-0 h-full w-3/4 sm:max-w-sm';

export const alertDialogContentClasses =
  'fixed left-[50%] top-[50%] z-50 grid w-full max-w-lg translate-x-[-50%] translate-y-[-50%] gap-4 border border-border bg-background p-6 shadow-lg sm:rounded-lg';

export const collapsibleContentClasses =
  'overflow-hidden text-sm';

export const dropdownContentClasses =
  'z-50 min-w-[8rem] overflow-hidden rounded-md border border-border bg-popover p-1 text-popover-foreground shadow-md';

export const dropdownItemClasses =
  'relative flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-sm outline-none transition-colors focus:bg-accent focus:text-accent-foreground data-[disabled]:pointer-events-none data-[disabled]:opacity-50';
