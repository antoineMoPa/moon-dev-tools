import { useTheme } from "../theme";

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
  const repoLabel = formatRepoLabel(repoName, branchName);
  const nextTheme = theme === "dark" ? "light" : "dark";

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
        {reviewLabel ? <div className="header-review-label">{reviewLabel}</div> : null}
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
