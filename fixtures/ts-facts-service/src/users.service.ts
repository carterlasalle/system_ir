import { Injectable } from "@nestjs/common";

@Injectable()
export class UsersService {
  private readonly table: string = "users";
  retries = 0;

  constructor() {}

  all(): string[] {
    return [this.table];
  }

  find(id: string): string | null {
    return id && this.retries >= 0 ? id : null;
  }
}
