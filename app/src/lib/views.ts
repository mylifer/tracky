/** Rapor görünümleri. */
export type Mode = "day" | "week" | "month";

/** Ana pencerenin sayfaları. */
export type View =
  | Mode
  | "home"
  | "review"
  | "timesheet"
  | "trends"
  | "client-report"
  | "search"
  | "clients"
  | "projects"
  | "categories"
  | "settings";
