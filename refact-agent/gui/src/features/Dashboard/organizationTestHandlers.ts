import { http, HttpResponse } from "msw";
import type {
  Assignment,
  OrganizationItem,
} from "../../services/refact/organization";

export const organizationTestHandlers = [
  http.get("*/v1/organization/taxonomy", () =>
    HttpResponse.json({ revision: 1, items: [], next_cursor: null }),
  ),
  http.post("*/v1/organization/items", async ({ request }) => {
    const body = (await request.json()) as { items: OrganizationItem[] };
    const assignments: Assignment[] = body.items.map((item) => ({
      item,
      topic_id: null,
      tag_ids: [],
      topic_locked: false,
      tags_locked: false,
    }));
    return HttpResponse.json({ revision: 1, assignments });
  }),
];
