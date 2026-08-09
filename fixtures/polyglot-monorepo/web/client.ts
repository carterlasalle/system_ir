/** API client for the payment service. */
import axios from "axios";

export const api = axios.create({ baseURL: "http://localhost:8080" });
