// Package schema checks expr values against the small part of JSON Schema
// the task schemas use: type (one or a list), properties, required,
// additionalProperties: false, items, maxItems, enum, const, oneOf.
//
// "integer" means an expr int (Go int): a float with no fraction is still a
// float, and times are ints in this dialect.
package schema

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"
)

type Schema struct {
	Type                 any                `json:"type"`
	Properties           map[string]*Schema `json:"properties"`
	Required             []string           `json:"required"`
	AdditionalProperties *bool              `json:"additionalProperties"`
	Items                *Schema            `json:"items"`
	MaxItems             *int               `json:"maxItems"`
	Enum                 []any              `json:"enum"`
	Const                any                `json:"const"`
	OneOf                []*Schema          `json:"oneOf"`
}

func Load(path string) (*Schema, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var s Schema
	if err := json.Unmarshal(b, &s); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return &s, nil
}

func (s *Schema) Validate(v any) error { return s.check("view", v) }

func (s *Schema) types() []string {
	switch t := s.Type.(type) {
	case string:
		return []string{t}
	case []any:
		var out []string
		for _, x := range t {
			out = append(out, x.(string))
		}
		return out
	}
	return nil
}

func typeOf(v any) string {
	switch v.(type) {
	case nil:
		return "null"
	case bool:
		return "boolean"
	case int:
		return "integer"
	case float64:
		return "number"
	case string:
		return "string"
	case []any:
		return "array"
	case map[string]any:
		return "object"
	}
	return fmt.Sprintf("%T", v)
}

func (s *Schema) check(path string, v any) error {
	if len(s.OneOf) > 0 {
		var errs []string
		for _, alt := range s.OneOf {
			err := alt.check(path, v)
			if err == nil {
				return nil
			}
			errs = append(errs, err.Error())
		}
		return fmt.Errorf("%s: matches no alternative (%s)", path, strings.Join(errs, "; "))
	}
	if ts := s.types(); ts != nil {
		got := typeOf(v)
		match := false
		for _, t := range ts {
			if t == got || (t == "number" && got == "integer") {
				match = true
			}
		}
		if !match {
			return fmt.Errorf("%s: want %s, got %s (%v)", path, strings.Join(ts, " or "), got, v)
		}
	}
	if s.Const != nil && fmt.Sprint(s.Const) != fmt.Sprint(v) {
		return fmt.Errorf("%s: want %v, got %v", path, s.Const, v)
	}
	if len(s.Enum) > 0 {
		found := false
		for _, e := range s.Enum {
			if fmt.Sprint(e) == fmt.Sprint(v) && (e == nil) == (v == nil) {
				found = true
			}
		}
		if !found {
			return fmt.Errorf("%s: %v is not one of %v", path, v, s.Enum)
		}
	}
	switch x := v.(type) {
	case map[string]any:
		for _, r := range s.Required {
			if _, ok := x[r]; !ok {
				return fmt.Errorf("%s: missing %q", path, r)
			}
		}
		keys := make([]string, 0, len(x))
		for k := range x {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			p, ok := s.Properties[k]
			if !ok {
				if s.AdditionalProperties != nil && !*s.AdditionalProperties {
					return fmt.Errorf("%s: unexpected field %q", path, k)
				}
				continue
			}
			if err := p.check(path+"."+k, x[k]); err != nil {
				return err
			}
		}
	case []any:
		if s.MaxItems != nil && len(x) > *s.MaxItems {
			return fmt.Errorf("%s: %d items, at most %d allowed", path, len(x), *s.MaxItems)
		}
		if s.Items != nil {
			for i, e := range x {
				if err := s.Items.check(fmt.Sprintf("%s[%d]", path, i), e); err != nil {
					return err
				}
			}
		}
	}
	return nil
}
