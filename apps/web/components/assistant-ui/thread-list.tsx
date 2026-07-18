"use client";

import {
  ThreadListItemMorePrimitive,
  ThreadListItemPrimitive,
  ThreadListPrimitive,
} from "@assistant-ui/react";
import { Archive, MessageSquare, MoreHorizontal, SquarePen, Trash2 } from "lucide-react";

function ThreadListItem() {
  return (
    <ThreadListItemPrimitive.Root className="aui-thread-list-item">
      <ThreadListItemPrimitive.Trigger className="aui-thread-list-trigger">
        <MessageSquare />
        <ThreadListItemPrimitive.Title fallback="New chat" />
      </ThreadListItemPrimitive.Trigger>
      <ThreadListItemMorePrimitive.Root>
        <ThreadListItemMorePrimitive.Trigger className="aui-thread-more" aria-label="Conversation actions"><MoreHorizontal /></ThreadListItemMorePrimitive.Trigger>
        <ThreadListItemMorePrimitive.Content className="aui-thread-menu">
          <ThreadListItemPrimitive.Archive asChild>
            <ThreadListItemMorePrimitive.Item><Archive />Archive</ThreadListItemMorePrimitive.Item>
          </ThreadListItemPrimitive.Archive>
          <ThreadListItemPrimitive.Delete asChild>
            <ThreadListItemMorePrimitive.Item className="danger-menu"><Trash2 />Delete</ThreadListItemMorePrimitive.Item>
          </ThreadListItemPrimitive.Delete>
        </ThreadListItemMorePrimitive.Content>
      </ThreadListItemMorePrimitive.Root>
    </ThreadListItemPrimitive.Root>
  );
}

export function ShennongThreadList() {
  return (
    <ThreadListPrimitive.Root className="aui-thread-list">
      <ThreadListPrimitive.New className="nav-item aui-new-thread"><SquarePen /><span>New chat</span></ThreadListPrimitive.New>
      <span className="sidebar-history-label">Chats</span>
      <ThreadListPrimitive.Items>{() => <ThreadListItem />}</ThreadListPrimitive.Items>
    </ThreadListPrimitive.Root>
  );
}
