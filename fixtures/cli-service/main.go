// cli-service: demo CLI with two subcommands, flags, and store writes.

package main

import (
	"fmt"

	"github.com/spf13/cobra"
)

var rootCmd = &cobra.Command{Use: "cli-service", Short: "demo CLI service"}

var serveCmd = &cobra.Command{
	Use:   "serve",
	Short: "serve requests",
	Run: func(cmd *cobra.Command, args []string) {
		port, _ := cmd.Flags().GetInt("port")
		fmt.Println(port)
	},
}

var deployCmd = &cobra.Command{
	Use:   "deploy",
	Short: "deploy the build",
	Run: func(cmd *cobra.Command, args []string) {
		env, _ := cmd.Flags().GetString("env")
		fmt.Println(env)
	},
}

func init() {
	rootCmd.AddCommand(serveCmd, deployCmd)
	serveCmd.Flags().IntP("port", "p", 8080, "port to listen on")
	serveCmd.Flags().Bool("paging", false, "enable paged output")
	deployCmd.Flags().StringP("env", "e", "dev", "target environment")
}

func main() {
	rootCmd.Execute()
}
