"use client";

import Link from "next/link";
import { ChevronsRight, Plus, Server } from "lucide-react";
import { Fragment, useMemo, useState } from "react";
import { useStore } from "@/lib/store";
import {
  buildProjectList,
  groupProjectsByRepo,
  projectHue,
  projectInitials,
  type ProjectEntry,
} from "@/lib/projectList";
import { CreateInstanceModal } from "./CreateInstanceModal";

interface SidebarRailProps {
  onExpand: () => void;
}

/**
 * The collapsed sidebar: one icon per project, Slack-workspace style, so a
 * folded sidebar still lets you see and switch projects. It lists the same
 * projects in the same order as the expanded `Sidebar` (repo groups, no-remote
 * bucket last, active first within a group) because both derive the list from
 * `lib/projectList.ts`.
 */
export function SidebarRail({ onExpand }: SidebarRailProps) {
  const { cliClients, sessions, machines, attachSession, sessionId } = useStore();
  const unreadSessions = useStore((s) => s.unreadSessions);
  const [newProjectOpen, setNewProjectOpen] = useState(false);

  const projects = useMemo(
    () => buildProjectList(sessions, cliClients, machines),
    [cliClients, sessions, machines],
  );
  const repoGroups = useMemo(() => groupProjectsByRepo(projects), [projects]);

  return (
    <nav
      aria-label="Projects"
      className="hidden h-full w-14 flex-shrink-0 flex-col items-center border-r border-gray-200 bg-gray-50 md:flex dark:border-gray-700 dark:bg-gray-800"
    >
      <div className="flex w-full flex-shrink-0 justify-center border-b border-gray-200 py-2 dark:border-gray-700">
        <button
          type="button"
          onClick={onExpand}
          className="rounded-lg p-2 text-gray-600 hover:bg-gray-200 dark:text-gray-300 dark:hover:bg-gray-700"
          title="Expand sidebar"
          aria-label="Expand sidebar"
        >
          <ChevronsRight className="h-5 w-5" />
        </button>
      </div>

      {/* The rail is the project list; hide its scrollbar so the icons stay
          centred in a 56px column. */}
      <div className="flex w-full flex-1 flex-col items-center gap-2 overflow-y-auto py-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {repoGroups.map((group, index) => (
          <Fragment key={group.key}>
            {index > 0 && (
              <div
                role="separator"
                aria-label={`${group.label} projects`}
                className="my-0.5 h-px w-6 flex-shrink-0 bg-gray-300 dark:bg-gray-600"
              />
            )}
            {group.projects.map((project) => (
              <ProjectIcon
                key={project.id}
                project={project}
                selected={sessionId === project.id}
                unread={unreadSessions.has(project.id) && sessionId !== project.id}
                onOpen={() => attachSession(project.id)}
              />
            ))}
          </Fragment>
        ))}
        <button
          type="button"
          onClick={() => setNewProjectOpen(true)}
          className="flex h-10 w-10 flex-shrink-0 items-center justify-center rounded-xl border border-dashed border-gray-300 text-gray-500 hover:border-emerald-500 hover:bg-emerald-50 hover:text-emerald-700 dark:border-gray-600 dark:text-gray-400 dark:hover:border-emerald-400 dark:hover:bg-emerald-950/40 dark:hover:text-emerald-300"
          title="Create project from GitHub"
          aria-label="Create project from GitHub"
        >
          <Plus className="h-5 w-5" />
        </button>
      </div>

      {/* Same destination as the expanded sidebar's "My Cluster" footer. */}
      <div className="flex w-full flex-shrink-0 justify-center border-t border-gray-200 py-2 dark:border-gray-700">
        <Link
          href="/machines"
          className="rounded-lg p-2 text-gray-600 hover:bg-gray-200 dark:text-gray-400 dark:hover:bg-gray-700"
          title="My Cluster"
          aria-label="My Cluster"
        >
          <Server className="h-5 w-5" />
        </Link>
      </div>

      {newProjectOpen && (
        <CreateInstanceModal open onClose={() => setNewProjectOpen(false)} />
      )}
    </nav>
  );
}

function ProjectIcon({
  project,
  selected,
  unread,
  onOpen,
}: {
  project: ProjectEntry;
  selected: boolean;
  unread: boolean;
  onOpen: () => void;
}) {
  // The hover text carries everything the expanded row shows, since the icon
  // itself can only fit two letters.
  const tooltip = [
    project.name,
    project.hostname,
    project.workingDir !== project.name ? project.workingDir : undefined,
    project.isShared && project.ownerEmail ? `Shared by ${project.ownerEmail}` : undefined,
    project.isActive ? "Active" : undefined,
    unread ? "New activity" : undefined,
  ]
    .filter(Boolean)
    .join("\n");

  return (
    <button
      type="button"
      onClick={onOpen}
      title={tooltip}
      aria-label={`Open ${project.name}`}
      aria-current={selected ? "page" : undefined}
      // Colour keyed on the stable project id, so the same project keeps its
      // colour across sessions; two same-named projects still differ.
      style={{ backgroundColor: `hsl(${projectHue(project.projectId)} 55% 45%)` }}
      className={`relative flex h-10 w-10 flex-shrink-0 items-center justify-center text-sm font-bold text-white transition-[border-radius,box-shadow] duration-150 hover:rounded-lg ${
        selected
          ? "rounded-lg ring-2 ring-blue-500 ring-offset-2 ring-offset-gray-50 dark:ring-offset-gray-800"
          : "rounded-xl"
      }`}
    >
      <span aria-hidden="true">{projectInitials(project.name)}</span>
      {project.isActive && (
        <span
          data-testid="rail-active-dot"
          aria-hidden="true"
          className="absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-gray-50 bg-green-500 dark:border-gray-800"
        />
      )}
      {unread && (
        <span
          data-testid="rail-unread-dot"
          aria-hidden="true"
          className="absolute -right-0.5 -top-0.5 h-3 w-3 animate-pulse rounded-full border-2 border-gray-50 bg-blue-500 dark:border-gray-800"
        />
      )}
    </button>
  );
}
