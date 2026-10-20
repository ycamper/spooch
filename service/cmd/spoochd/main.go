// spoochd is Spooch's sharing service.
//
//	spoochd                       serve (the default)
//	spoochd user add NAME         add a user
//	spoochd token add NAME LABEL  give NAME's device LABEL a token (shown once)
//	spoochd token list            list devices
//	spoochd token revoke ID       stop a device's token working
//
// Configuration is from the environment; see service/README.md.
package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"syscall"
	"time"

	"github.com/ycamper/spooch/service/internal/api"
	"github.com/ycamper/spooch/service/internal/storage"
	"github.com/ycamper/spooch/service/internal/store"
	"github.com/ycamper/spooch/service/internal/token"
)

func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, "spoochd:", err)
		os.Exit(1)
	}
}

func env(name, fallback string) string {
	if v := os.Getenv(name); v != "" {
		return v
	}
	return fallback
}

func need(name string) (string, error) {
	if v := os.Getenv(name); v != "" {
		return v, nil
	}
	return "", fmt.Errorf("%s isn't set", name)
}

func run(args []string) error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	dbURL, err := need("SPOOCH_DATABASE_URL")
	if err != nil {
		return err
	}
	db, err := store.Open(ctx, dbURL)
	if err != nil {
		return fmt.Errorf("database: %w", err)
	}
	defer db.Close()
	if len(args) > 0 {
		return admin(ctx, db, args)
	}
	return serve(ctx, db)
}

func admin(ctx context.Context, db store.Store, args []string) error {
	usage := errors.New("usage: spoochd [user add NAME | token add NAME LABEL | token list | token revoke ID]")
	switch {
	case len(args) == 3 && args[0] == "user" && args[1] == "add":
		u, err := db.AddUser(ctx, args[2])
		if err != nil {
			return err
		}
		fmt.Printf("added user %s (id %d)\n", u.Name, u.ID)
	case len(args) == 4 && args[0] == "token" && args[1] == "add":
		u, err := db.UserByName(ctx, args[2])
		if err != nil {
			return fmt.Errorf("user %s: %w", args[2], err)
		}
		t := token.NewDevice()
		d, err := db.AddDevice(ctx, u.ID, args[3], token.Hash(t))
		if err != nil {
			return err
		}
		fmt.Printf("device %d (%s, %s). Its token, shown only now:\n%s\n", d.ID, d.User, d.Label, t)
	case len(args) == 2 && args[0] == "token" && args[1] == "list":
		devices, err := db.Devices(ctx)
		if err != nil {
			return err
		}
		for _, d := range devices {
			used := "never used"
			if d.LastUsed != nil {
				used = "last used " + d.LastUsed.Format(time.DateTime)
			}
			state := ""
			if d.Revoked {
				state = ", revoked"
			}
			fmt.Printf("%d\t%s\t%s\t%s%s\n", d.ID, d.User, d.Label, used, state)
		}
	case len(args) == 3 && args[0] == "token" && args[1] == "revoke":
		id, err := strconv.ParseInt(args[2], 10, 64)
		if err != nil {
			return usage
		}
		if err := db.RevokeDevice(ctx, id); err != nil {
			return err
		}
		fmt.Printf("revoked device %d\n", id)
	default:
		return usage
	}
	return nil
}

func serve(ctx context.Context, db store.Store) error {
	key, err := need("SPOOCH_S3_KEY")
	if err != nil {
		return err
	}
	secret, err := need("SPOOCH_S3_SECRET")
	if err != nil {
		return err
	}
	files, err := storage.NewS3(storage.S3Config{
		Endpoint:       env("SPOOCH_S3_ENDPOINT", "localhost:9000"),
		PublicEndpoint: os.Getenv("SPOOCH_S3_PUBLIC_ENDPOINT"),
		Region:         env("SPOOCH_S3_REGION", "us-east-1"),
		Bucket:         env("SPOOCH_S3_BUCKET", "spooch"),
		Key:            key,
		Secret:         secret,
		Secure:         env("SPOOCH_S3_SECURE", "true") == "true",
	})
	if err != nil {
		return fmt.Errorf("storage: %w", err)
	}
	quota, err := strconv.ParseInt(env("SPOOCH_QUOTA_GB", "10"), 10, 64)
	if err != nil || quota < 0 {
		return fmt.Errorf("SPOOCH_QUOTA_GB must be a whole number of GB")
	}
	srv := &api.Server{
		Store:      db,
		Storage:    files,
		PublicURL:  env("SPOOCH_PUBLIC_URL", "http://localhost:8080"),
		Now:        time.Now,
		SignupCode: os.Getenv("SPOOCH_SIGNUP_CODE"),
		Quota:      quota << 30,
	}
	addr := env("SPOOCH_ADDR", ":8080")
	httpd := &http.Server{
		Addr:              addr,
		Handler:           srv.Handler(),
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       15 * time.Second,
		// Room to send a 25 MB image over a slow connection.
		WriteTimeout:   2 * time.Minute,
		IdleTimeout:    2 * time.Minute,
		MaxHeaderBytes: 16 << 10,
	}
	swept := make(chan struct{})
	go func() {
		defer close(swept)
		sweep(ctx, srv)
	}()
	stopped := make(chan error, 1)
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		stopped <- httpd.Shutdown(shutdown)
	}()
	log.Printf("spoochd on %s, links at %s", addr, srv.PublicURL)
	if err := httpd.ListenAndServe(); !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	// Requests under way finish before the database closes.
	err = <-stopped
	<-swept
	return err
}

// sweep clears out expired and abandoned shares, and captures a month in
// the trash or never finished, every ten minutes.
func sweep(ctx context.Context, srv *api.Server) {
	tick := time.NewTicker(10 * time.Minute)
	defer tick.Stop()
	for {
		if shares, captures, err := srv.Sweep(ctx); err != nil {
			log.Printf("sweep: %v", err)
		} else if shares+captures > 0 {
			log.Printf("swept %d shares, %d captures", shares, captures)
		}
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
		}
	}
}
