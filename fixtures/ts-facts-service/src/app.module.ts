import { Controller, Get, Module } from "@nestjs/common";
import { UsersService } from "./users.service";

@Controller("users")
export class UsersController {
  constructor(private readonly users: UsersService) {}

  @Get()
  async list(): Promise<string[]> {
    return this.users.all();
  }

  @Get(":id")
  async get(id: string): Promise<string | null> {
    return this.users.find(id);
  }
}

@Module({
  controllers: [UsersController],
  providers: [UsersService],
})
export class AppModule {}
