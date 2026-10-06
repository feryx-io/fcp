from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="BrandColors")



@_attrs_define
class BrandColors:
    """ The colors of one theme, each `#RRGGBB` or `#RRGGBBAA`; one unset is
    the app's own.

        Attributes:
            primary (None | str | Unset):
            accent (None | str | Unset):
            background (None | str | Unset):
            surface (None | str | Unset):
            on_primary (None | str | Unset):
            on_surface (None | str | Unset):
            error (None | str | Unset):
     """

    primary: None | str | Unset = UNSET
    accent: None | str | Unset = UNSET
    background: None | str | Unset = UNSET
    surface: None | str | Unset = UNSET
    on_primary: None | str | Unset = UNSET
    on_surface: None | str | Unset = UNSET
    error: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        primary: None | str | Unset
        if isinstance(self.primary, Unset):
            primary = UNSET
        else:
            primary = self.primary

        accent: None | str | Unset
        if isinstance(self.accent, Unset):
            accent = UNSET
        else:
            accent = self.accent

        background: None | str | Unset
        if isinstance(self.background, Unset):
            background = UNSET
        else:
            background = self.background

        surface: None | str | Unset
        if isinstance(self.surface, Unset):
            surface = UNSET
        else:
            surface = self.surface

        on_primary: None | str | Unset
        if isinstance(self.on_primary, Unset):
            on_primary = UNSET
        else:
            on_primary = self.on_primary

        on_surface: None | str | Unset
        if isinstance(self.on_surface, Unset):
            on_surface = UNSET
        else:
            on_surface = self.on_surface

        error: None | str | Unset
        if isinstance(self.error, Unset):
            error = UNSET
        else:
            error = self.error


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if primary is not UNSET:
            field_dict["primary"] = primary
        if accent is not UNSET:
            field_dict["accent"] = accent
        if background is not UNSET:
            field_dict["background"] = background
        if surface is not UNSET:
            field_dict["surface"] = surface
        if on_primary is not UNSET:
            field_dict["on_primary"] = on_primary
        if on_surface is not UNSET:
            field_dict["on_surface"] = on_surface
        if error is not UNSET:
            field_dict["error"] = error

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_primary(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        primary = _parse_primary(d.pop("primary", UNSET))


        def _parse_accent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        accent = _parse_accent(d.pop("accent", UNSET))


        def _parse_background(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        background = _parse_background(d.pop("background", UNSET))


        def _parse_surface(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        surface = _parse_surface(d.pop("surface", UNSET))


        def _parse_on_primary(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        on_primary = _parse_on_primary(d.pop("on_primary", UNSET))


        def _parse_on_surface(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        on_surface = _parse_on_surface(d.pop("on_surface", UNSET))


        def _parse_error(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        error = _parse_error(d.pop("error", UNSET))


        brand_colors = cls(
            primary=primary,
            accent=accent,
            background=background,
            surface=surface,
            on_primary=on_primary,
            on_surface=on_surface,
            error=error,
        )


        brand_colors.additional_properties = d
        return brand_colors

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
