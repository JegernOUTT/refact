import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { organizationApi } from "../../services/refact/organization";

describe("organization API snapshots", () => {
  it("restarts stale taxonomy pagination without merging revisions", async () => {
    let calls = 0;
    server.use(
      http.get("*/v1/organization/taxonomy", ({ request }) => {
        calls++;
        const cursor = new URL(request.url).searchParams.get("cursor");
        if (calls === 2) return new HttpResponse(null, { status: 409 });
        return HttpResponse.json({
          revision: calls === 1 ? 1 : 2,
          items: [
            {
              id: cursor ? "second" : calls === 1 ? "stale" : "first",
              name: "Topic",
              archived: false,
            },
          ],
          next_cursor: cursor ? null : "next",
        });
      }),
    );
    const store = setUpStore();
    const query = store.dispatch(
      organizationApi.endpoints.organizationTaxonomy.initiate({
        scope: "project",
        kind: "topics",
      }),
    );
    const data = await query.unwrap();
    expect(data.revision).toBe(2);
    expect(data.items.map((item) => item.id)).toEqual(["first", "second"]);
    expect(calls).toBe(4);
    query.unsubscribe();
  });
  it("bounds stale taxonomy retries", async () => {
    let calls = 0;
    server.use(
      http.get("*/v1/organization/taxonomy", () => {
        calls++;
        return new HttpResponse(null, { status: 409 });
      }),
    );
    const query = setUpStore().dispatch(
      organizationApi.endpoints.organizationTaxonomy.initiate({
        scope: "project",
        kind: "tags",
      }),
    );
    await expect(query.unwrap()).rejects.toMatchObject({
      status: "CUSTOM_ERROR",
    });
    expect(calls).toBe(3);
    query.unsubscribe();
  });
  it("requests only supplied canonical keys in batches no larger than 200", async () => {
    const sizes: number[] = [];
    server.use(
      http.post("*/v1/organization/items", async ({ request }) => {
        const body = (await request.json()) as {
          items: { kind: string; id: string }[];
        };
        sizes.push(body.items.length);
        return HttpResponse.json({
          revision: 4,
          assignments: body.items.map((item) => ({
            item,
            topic_id: null,
            tag_ids: [],
            topic_locked: false,
            tags_locked: false,
          })),
        });
      }),
    );
    const query = setUpStore().dispatch(
      organizationApi.endpoints.organizationItems.initiate({
        scope: "project",
        items: Array.from({ length: 401 }, (_, id) => ({
          kind: "chat",
          id: String(id),
        })),
      }),
    );
    expect((await query.unwrap()).assignments).toHaveLength(401);
    expect(sizes).toEqual([200, 200, 1]);
    query.unsubscribe();
  });
  it("isolates forbidden keys without dropping authorized assignments", async () => {
    const requests: string[][] = [];
    server.use(
      http.post("*/v1/organization/items", async ({ request }) => {
        const { items } = (await request.json()) as {
          items: { kind: string; id: string }[];
        };
        requests.push(items.map((item) => item.id));
        if (items.some((item) => item.id === "orphan"))
          return new HttpResponse(null, { status: 403 });
        return HttpResponse.json({
          revision: 7,
          assignments: items.map((item) => ({
            item,
            topic_id: "topic",
            tag_ids: ["tag"],
            topic_locked: false,
            tags_locked: false,
          })),
        });
      }),
    );
    const query = setUpStore().dispatch(
      organizationApi.endpoints.organizationItems.initiate({
        scope: "mixed",
        items: [
          { kind: "chat", id: "root" },
          { kind: "chat", id: "orphan" },
        ],
      }),
    );
    const data = await query.unwrap();
    expect(data.assignments.map((assignment) => assignment.item.id)).toEqual([
      "root",
    ]);
    expect(data.unavailable_items).toEqual([{ kind: "chat", id: "orphan" }]);
    expect(requests).toEqual([["root", "orphan"], ["root"], ["orphan"]]);
    query.unsubscribe();
  });
  it("caps extra isolation calls for large denied history", async () => {
    let calls = 0;
    server.use(
      http.post("*/v1/organization/items", () => {
        calls++;
        return new HttpResponse(null, { status: 403 });
      }),
    );
    const query = setUpStore().dispatch(
      organizationApi.endpoints.organizationItems.initiate({
        scope: "denied",
        items: Array.from({ length: 5000 }, (_, id) => ({
          kind: "chat",
          id: String(id),
        })),
      }),
    );
    const data = await query.unwrap();
    expect(data.unavailable_items).toHaveLength(5000);
    expect(data.assignments).toEqual([]);
    expect(calls).toBe(25 + 32);
    query.unsubscribe();
  });
  it("never retries or overwrites a revision conflict", async () => {
    let calls = 0;
    server.use(
      http.post("*/v1/organization/apply", () => {
        calls++;
        return new HttpResponse(null, { status: 409 });
      }),
    );
    const mutation = setUpStore().dispatch(
      organizationApi.endpoints.applyOrganization.initiate({
        scope: "project",
        expected_revision: 1,
        request_id: "manual-request",
        operations: [
          {
            op: "assign_item",
            item: { kind: "chat", id: "original-root" },
            topic_id: "topic",
            tag_ids: [],
            topic_locked: true,
            tags_locked: false,
          },
        ],
      }),
    );
    await expect(mutation.unwrap()).rejects.toMatchObject({ status: 409 });
    expect(calls).toBe(1);
  });
});
