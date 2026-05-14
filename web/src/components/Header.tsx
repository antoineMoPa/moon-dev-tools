import { useTheme } from "../theme";
import { useOptionalReviewStore } from "../reviewStore";

type HeaderProps = {
  repoName?: string | null;
  branchName?: string | null;
  reviewLabel?: string | null;
};

function formatRepoLabel(repoName?: string | null, branchName?: string | null): string | null {
  if (!repoName) {
    return null;
  }

  return branchName ? `${repoName} / ${branchName}` : repoName;
}

export function Header({ repoName, branchName, reviewLabel }: HeaderProps) {
  const { theme, toggleTheme } = useTheme();
  const reviewStore = useOptionalReviewStore();
  const movedDiffLayout = reviewStore?.state.movedDiffLayout ?? "unified";
  const repoLabel = formatRepoLabel(repoName, branchName);
  const nextTheme = theme === "dark" ? "light" : "dark";
  const nextMovedDiffLayout = movedDiffLayout === "side-by-side" ? "unified" : "side-by-side";

  return (
    <header>
      <div className="header-inner">
        <div>
          <h1>
            <a
              className="header-title-link"
              href="https://github.com/antoineMoPa/moonreview"
              target="_blank"
              rel="noreferrer"
            >
              🌚 moonreview
            </a>
          </h1>
        </div>
        <div className="header-center">
          {reviewLabel ? <div className="header-review-label">{reviewLabel}</div> : null}
          {reviewStore ? (
            <button
              type="button"
              className="header-move-layout-toggle"
              onClick={() => reviewStore.actions.setMovedDiffLayout(nextMovedDiffLayout)}
              title="Toggle moved-code diff layout"
            >
              [{movedDiffLayout === "side-by-side" ? "unified" : "side by side"}]
            </button>
          ) : null}
        </div>
        <div className="header-actions">
          {repoLabel ? <div className="header-repo-name">{repoLabel}</div> : null}
          <button
            type="button"
            className="theme-toggle"
            onClick={toggleTheme}
            aria-label={`Switch to ${nextTheme} mode`}
            title={`Switch to ${nextTheme} mode`}
          >
            <span className="theme-toggle-icon" aria-hidden="true">
              {theme === "dark" ? "☀" : "☾"}
            </span>
            <span>{theme === "dark" ? "Light" : "Dark"}</span>
          </button>
        </div>
      </div>
    </header>
  );
}
