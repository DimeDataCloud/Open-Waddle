// Which AI service the planner and the quick replies use. The planner's service is the
// "preset"; quick replies can sit on their own service (say Google's free Gemini API next to
// OpenRouter for the planner), with their own key.

export const GOOGLE_URL = "https://generativelanguage.googleapis.com/v1beta/openai/";
export const GOOGLE_PLANNER = "gemini-3.8-flash";
export const GOOGLE_QUICK = "gemini-3.5-flash-lite";

export type FastService = "same" | "google" | "custom";

/** Where quick replies run, from the saved URL: empty is the planner's service. */
export function fastServiceFor(fastBaseUrl: string): FastService {
  const url = fastBaseUrl.trim();
  if (!url) return "same";
  return url.includes("generativelanguage.googleapis.com") ? "google" : "custom";
}

/** What to fill in when the quick-reply service is changed. `plannerFast` is the quick-reply
 *  model the planner's own preset suggests (used when going back to "same"). */
export function fastDefaults(service: FastService, plannerFast: string, currentUrl: string, currentModel: string): { url: string; model: string } {
  switch (service) {
    case "same":
      return { url: "", model: plannerFast };
    case "google":
      return { url: GOOGLE_URL, model: currentModel.startsWith("gemini-") ? currentModel : GOOGLE_QUICK };
    case "custom":
      // Keep what's there, except Google's address, which is its own choice.
      return { url: currentUrl.includes("generativelanguage.googleapis.com") ? "" : currentUrl, model: currentModel };
  }
}

/** Whether to show the note about Google's free tier: either part is on Google. */
export function usesGoogle(presetUrl: string, fastBaseUrl: string): boolean {
  return presetUrl.includes("generativelanguage.googleapis.com") || fastServiceFor(fastBaseUrl) === "google";
}
