// Matches the backend `Paginated<T>` envelope ({ items, total, page, pageSize }).
export interface Paginated<T> {
  items: T[];
  total: number;
  page: number;
  pageSize: number;
}

export const DEFAULT_PAGE_SIZE = 20;
