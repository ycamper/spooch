// Package storage holds the shared images: an S3-compatible bucket
// (DigitalOcean Spaces), or memory in tests. The app uploads straight to
// it through a presigned PUT.
package storage

import (
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"net/url"
	"strconv"
	"sync"
	"time"

	"github.com/minio/minio-go/v7"
	"github.com/minio/minio-go/v7/pkg/credentials"
)

var ErrNotFound = errors.New("no such object")

type Storage interface {
	// PresignPut is a URL the app can PUT `key` to until `ttl` passes,
	// with exactly `contentType` and `size` bytes.
	PresignPut(ctx context.Context, key, contentType string, size int64, ttl time.Duration) (string, error)
	// PresignGet is a URL anyone may GET `key` from until `ttl` passes.
	PresignGet(ctx context.Context, key string, ttl time.Duration) (string, error)
	// Size is how big `key` is.
	Size(ctx context.Context, key string) (int64, error)
	// Open reads `key`, or is ErrNotFound before anything is read.
	Open(ctx context.Context, key string) (io.ReadCloser, error)
	Copy(ctx context.Context, from, to string) error
	Delete(ctx context.Context, key string) error
}

// S3 is a bucket on an S3-compatible service.
type S3 struct {
	bucket string
	client *minio.Client
	// public signs upload URLs for the address the app reaches, if that
	// differs from the one the service uses.
	public *minio.Client
}

type S3Config struct {
	Endpoint       string // host:port the service talks to
	PublicEndpoint string // host:port the app uploads to; Endpoint if empty
	Region         string
	Bucket         string
	Key, Secret    string
	Secure         bool
}

func NewS3(c S3Config) (*S3, error) {
	opts := func() *minio.Options {
		return &minio.Options{
			Creds:  credentials.NewStaticV4(c.Key, c.Secret, ""),
			Secure: c.Secure,
			Region: c.Region,
		}
	}
	client, err := minio.New(c.Endpoint, opts())
	if err != nil {
		return nil, err
	}
	public := client
	if c.PublicEndpoint != "" && c.PublicEndpoint != c.Endpoint {
		if public, err = minio.New(c.PublicEndpoint, opts()); err != nil {
			return nil, err
		}
	}
	return &S3{bucket: c.Bucket, client: client, public: public}, nil
}

func (s *S3) PresignPut(ctx context.Context, key, contentType string, size int64, ttl time.Duration) (string, error) {
	signed := http.Header{}
	signed.Set("Content-Type", contentType)
	signed.Set("Content-Length", strconv.FormatInt(size, 10))
	u, err := s.public.PresignHeader(ctx, http.MethodPut, s.bucket, key, ttl, url.Values{}, signed)
	if err != nil {
		return "", err
	}
	return u.String(), nil
}

func (s *S3) PresignGet(ctx context.Context, key string, ttl time.Duration) (string, error) {
	u, err := s.public.PresignedGetObject(ctx, s.bucket, key, ttl, url.Values{})
	if err != nil {
		return "", err
	}
	return u.String(), nil
}

// missing maps the bucket's "no such key" to ErrNotFound.
func missing(err error) error {
	if minio.ToErrorResponse(err).Code == "NoSuchKey" {
		return ErrNotFound
	}
	return err
}

func (s *S3) Size(ctx context.Context, key string) (int64, error) {
	info, err := s.client.StatObject(ctx, s.bucket, key, minio.StatObjectOptions{})
	if err != nil {
		return 0, missing(err)
	}
	return info.Size, nil
}

func (s *S3) Open(ctx context.Context, key string) (io.ReadCloser, error) {
	obj, err := s.client.GetObject(ctx, s.bucket, key, minio.GetObjectOptions{})
	if err != nil {
		return nil, missing(err)
	}
	// GetObject is lazy; Stat makes it reach the bucket now.
	if _, err := obj.Stat(); err != nil {
		obj.Close()
		return nil, missing(err)
	}
	return obj, nil
}

func (s *S3) Copy(ctx context.Context, from, to string) error {
	_, err := s.client.CopyObject(ctx,
		minio.CopyDestOptions{Bucket: s.bucket, Object: to},
		minio.CopySrcOptions{Bucket: s.bucket, Object: from})
	return missing(err)
}

func (s *S3) Delete(ctx context.Context, key string) error {
	return s.client.RemoveObject(ctx, s.bucket, key, minio.RemoveObjectOptions{})
}

// Memory is a Storage in memory, for tests; its upload URLs aren't real.
type Memory struct {
	mu      sync.Mutex
	objects map[string][]byte
	// Fail makes every call fail, to test outages.
	Fail error
}

func NewMemory() *Memory { return &Memory{objects: map[string][]byte{}} }

func (m *Memory) PresignPut(_ context.Context, key, _ string, _ int64, _ time.Duration) (string, error) {
	return "memory://upload/" + url.PathEscape(key), m.Fail
}

func (m *Memory) PresignGet(_ context.Context, key string, _ time.Duration) (string, error) {
	return "memory://get/" + url.PathEscape(key), m.Fail
}

// Put stands in for the app's upload.
func (m *Memory) Put(key string, data []byte) {
	m.mu.Lock()
	defer m.mu.Unlock()
	m.objects[key] = data
}

func (m *Memory) Has(key string) bool {
	m.mu.Lock()
	defer m.mu.Unlock()
	_, ok := m.objects[key]
	return ok
}

func (m *Memory) get(key string) ([]byte, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.Fail != nil {
		return nil, m.Fail
	}
	b, ok := m.objects[key]
	if !ok {
		return nil, ErrNotFound
	}
	return b, nil
}

func (m *Memory) Size(_ context.Context, key string) (int64, error) {
	b, err := m.get(key)
	return int64(len(b)), err
}

func (m *Memory) Open(_ context.Context, key string) (io.ReadCloser, error) {
	b, err := m.get(key)
	if err != nil {
		return nil, err
	}
	return io.NopCloser(bytes.NewReader(b)), nil
}

func (m *Memory) Copy(_ context.Context, from, to string) error {
	b, err := m.get(from)
	if err != nil {
		return err
	}
	m.Put(to, b)
	return nil
}

func (m *Memory) Delete(_ context.Context, key string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.Fail != nil {
		return m.Fail
	}
	delete(m.objects, key)
	return nil
}
