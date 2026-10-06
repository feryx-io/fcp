from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.brand_colors import BrandColors
  from ..models.brand_dial_plan import BrandDialPlan
  from ..models.brand_links import BrandLinks
  from ..models.brand_ringtone import BrandRingtone





T = TypeVar("T", bound="BrandRequest")



@_attrs_define
class BrandRequest:
    """ The tenant's brand, as its administrator writes it: assets by hash.

        Attributes:
            app_name (str): The app's name, as the tenant's people see it.
            colors (BrandColors | Unset): The colors of one theme, each `#RRGGBB` or `#RRGGBBAA`; one unset is
                the app's own.
            dark_colors (BrandColors | None | Unset):
            logo (None | str | Unset): An uploaded image's hash.
            logo_dark (None | str | Unset):
            icon (None | str | Unset): An uploaded PNG, JPEG or WebP image's hash.
            ringtones (list[BrandRingtone] | Unset): Uploaded audio, by hash; at most one the default.
            links (BrandLinks | Unset): The tenant's links, each `https://`.
            dial_plan (BrandDialPlan | Unset): Hints for the app's dialer: prefixes it takes off what is dialed, and
                short codes it turns into addresses.
     """

    app_name: str
    colors: BrandColors | Unset = UNSET
    dark_colors: BrandColors | None | Unset = UNSET
    logo: None | str | Unset = UNSET
    logo_dark: None | str | Unset = UNSET
    icon: None | str | Unset = UNSET
    ringtones: list[BrandRingtone] | Unset = UNSET
    links: BrandLinks | Unset = UNSET
    dial_plan: BrandDialPlan | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.brand_colors import BrandColors # noqa: PLC0415
        from ..models.brand_dial_plan import BrandDialPlan # noqa: PLC0415
        from ..models.brand_links import BrandLinks # noqa: PLC0415
        from ..models.brand_ringtone import BrandRingtone # noqa: PLC0415
        app_name = self.app_name

        colors: dict[str, Any] | Unset = UNSET
        if not isinstance(self.colors, Unset):
            colors = self.colors.to_dict()

        dark_colors: dict[str, Any] | None | Unset
        if isinstance(self.dark_colors, Unset):
            dark_colors = UNSET
        elif isinstance(self.dark_colors, BrandColors):
            dark_colors = self.dark_colors.to_dict()
        else:
            dark_colors = self.dark_colors

        logo: None | str | Unset
        if isinstance(self.logo, Unset):
            logo = UNSET
        else:
            logo = self.logo

        logo_dark: None | str | Unset
        if isinstance(self.logo_dark, Unset):
            logo_dark = UNSET
        else:
            logo_dark = self.logo_dark

        icon: None | str | Unset
        if isinstance(self.icon, Unset):
            icon = UNSET
        else:
            icon = self.icon

        ringtones: list[dict[str, Any]] | Unset = UNSET
        if not isinstance(self.ringtones, Unset):
            ringtones = []
            for ringtones_item_data in self.ringtones:
                ringtones_item = ringtones_item_data.to_dict()
                ringtones.append(ringtones_item)



        links: dict[str, Any] | Unset = UNSET
        if not isinstance(self.links, Unset):
            links = self.links.to_dict()

        dial_plan: dict[str, Any] | Unset = UNSET
        if not isinstance(self.dial_plan, Unset):
            dial_plan = self.dial_plan.to_dict()


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "app_name": app_name,
        })
        if colors is not UNSET:
            field_dict["colors"] = colors
        if dark_colors is not UNSET:
            field_dict["dark_colors"] = dark_colors
        if logo is not UNSET:
            field_dict["logo"] = logo
        if logo_dark is not UNSET:
            field_dict["logo_dark"] = logo_dark
        if icon is not UNSET:
            field_dict["icon"] = icon
        if ringtones is not UNSET:
            field_dict["ringtones"] = ringtones
        if links is not UNSET:
            field_dict["links"] = links
        if dial_plan is not UNSET:
            field_dict["dial_plan"] = dial_plan

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.brand_colors import BrandColors # noqa: PLC0415
        from ..models.brand_dial_plan import BrandDialPlan # noqa: PLC0415
        from ..models.brand_links import BrandLinks # noqa: PLC0415
        from ..models.brand_ringtone import BrandRingtone # noqa: PLC0415
        d = dict(src_dict)
        app_name = d.pop("app_name")

        _colors = d.pop("colors", UNSET)
        colors: BrandColors | Unset
        if isinstance(_colors,  Unset):
            colors = UNSET
        else:
            colors = BrandColors.from_dict(_colors)




        def _parse_dark_colors(data: object) -> BrandColors | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                dark_colors_type_0 = BrandColors.from_dict(data)



                return dark_colors_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(BrandColors | None | Unset, data)

        dark_colors = _parse_dark_colors(d.pop("dark_colors", UNSET))


        def _parse_logo(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        logo = _parse_logo(d.pop("logo", UNSET))


        def _parse_logo_dark(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        logo_dark = _parse_logo_dark(d.pop("logo_dark", UNSET))


        def _parse_icon(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        icon = _parse_icon(d.pop("icon", UNSET))


        _ringtones = d.pop("ringtones", UNSET)
        ringtones: list[BrandRingtone] | Unset = UNSET
        if _ringtones is not UNSET:
            ringtones = []
            for ringtones_item_data in _ringtones:
                ringtones_item = BrandRingtone.from_dict(ringtones_item_data)



                ringtones.append(ringtones_item)


        _links = d.pop("links", UNSET)
        links: BrandLinks | Unset
        if isinstance(_links,  Unset):
            links = UNSET
        else:
            links = BrandLinks.from_dict(_links)




        _dial_plan = d.pop("dial_plan", UNSET)
        dial_plan: BrandDialPlan | Unset
        if isinstance(_dial_plan,  Unset):
            dial_plan = UNSET
        else:
            dial_plan = BrandDialPlan.from_dict(_dial_plan)




        brand_request = cls(
            app_name=app_name,
            colors=colors,
            dark_colors=dark_colors,
            logo=logo,
            logo_dark=logo_dark,
            icon=icon,
            ringtones=ringtones,
            links=links,
            dial_plan=dial_plan,
        )


        brand_request.additional_properties = d
        return brand_request

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
