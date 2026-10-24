import type { EventTransport, FinancialEvent } from "../domain/types";
export class MockTransport implements EventTransport {
  private events = new Map<string, FinancialEvent>();
  async publish(events: FinancialEvent[]) {
    for (const event of events)
      this.events.set(event.id, structuredClone(event));
  }
  async synchronize(businessId: string) {
    return [...this.events.values()]
      .filter((e) => e.businessId === businessId)
      .map((e) => structuredClone(e));
  }
}
export const transport = new MockTransport();
