import { X, GitBranch } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useSettingsStore } from "../stores/settingsStore";

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
}

function ToggleSwitch({
  checked,
  onChange,
  label,
  description,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
}) {
  return (
    <label className="flex items-start gap-3 cursor-pointer group">
      <div className="pt-0.5">
        <button
          type="button"
          role="switch"
          aria-checked={checked}
          onClick={() => onChange(!checked)}
          className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 ${
            checked ? "bg-primary" : "bg-gray-600"
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
              checked ? "translate-x-4" : "translate-x-0"
            }`}
          />
        </button>
      </div>
      <div className="flex-1">
        <span className="text-sm font-medium text-text-primary group-hover:text-primary transition-colors">
          {label}
        </span>
        {description && (
          <p className="text-xs text-text-secondary mt-0.5">{description}</p>
        )}
      </div>
    </label>
  );
}

export function SettingsModal({ isOpen, onClose }: SettingsModalProps) {
  const { t } = useTranslation();
  const {
    showGitStatusIndicators,
    showBranchInfo,
    showAheadBehind,
    setShowGitStatusIndicators,
    setShowBranchInfo,
    setShowAheadBehind,
  } = useSettingsStore();

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-card-bg border border-border rounded-lg shadow-xl w-full max-w-md">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border">
          <h2 className="text-lg font-semibold">{t("settings.title")}</h2>
          <button
            onClick={onClose}
            className="p-1 rounded hover:bg-background text-text-secondary hover:text-text-primary transition-colors"
          >
            <X size={18} />
          </button>
        </div>

        {/* Content */}
        <div className="p-4 space-y-6">
          {/* Git Section */}
          <div>
            <div className="flex items-center gap-2 mb-3">
              <GitBranch size={16} className="text-primary" />
              <h3 className="text-sm font-semibold text-text-primary uppercase tracking-wide">
                {t("settings.gitSection")}
              </h3>
            </div>
            <div className="space-y-4 pl-6">
              <ToggleSwitch
                checked={showBranchInfo}
                onChange={setShowBranchInfo}
                label={t("settings.showBranchInfo")}
                description={t("settings.showBranchInfoDesc")}
              />
              <ToggleSwitch
                checked={showAheadBehind}
                onChange={setShowAheadBehind}
                label={t("settings.showAheadBehind")}
                description={t("settings.showAheadBehindDesc")}
              />
              <ToggleSwitch
                checked={showGitStatusIndicators}
                onChange={setShowGitStatusIndicators}
                label={t("settings.showGitStatusIndicators")}
                description={t("settings.showGitStatusIndicatorsDesc")}
              />
            </div>
          </div>
        </div>

        {/* Footer */}
        <div className="flex justify-end px-4 py-3 border-t border-border">
          <button
            onClick={onClose}
            className="px-4 py-2 text-sm font-medium bg-primary text-white rounded hover:bg-primary/90 transition-colors"
          >
            {t("common.done")}
          </button>
        </div>
      </div>
    </div>
  );
}
