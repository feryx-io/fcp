from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.brand_asset_response import BrandAssetResponse
  from ..models.brand_colors import BrandColors
  from ..models.brand_dial_plan import BrandDialPlan
  from ..models.brand_links import BrandLinks
  from ..models.brand_ringtone import BrandRingtone





T = TypeVar("T", bound="BrandResponse")



@_attrs_define
class BrandResponse:
    """ The tenant's brand, with what its assets are.

        Attributes:
            tenant_id (str):
            app_name (str):
            colors (BrandColors): The colors of one theme, each `#RRGGBB` or `#RRGGBBAA`; one unset is
                the app's own.
            ringtones (list[BrandRingtone]):
            links (BrandLinks): The tenant's links, each `https://`.
            dial_plan (BrandDialPlan): Hints for the app's dialer: prefixes it takes off what is dialed, and
                short codes it turns into addresses.
            assets (list[BrandAssetResponse]): The assets it names.
            updated_at (datetime.datetime):
            dark_colors (BrandColors | None | Unset):
            logo (None | str | Unset):
            logo_dark (None | str | Unset):
            icon (None | str | Unset):
            brand_url (None | str | Unset): Where the tenant's softphones fetch it.
     """

    tenant_id: str
    app_name: str
    colors: BrandColors
    ringtones: list[BrandRingtone]
    links: BrandLinks
    dial_plan: BrandDialPlan
    assets: list[BrandAssetResponse]
    updated_at: datetime.datetime
    dark_colors: BrandColors | None | Unset = UNSET
    logo: None | str | Unset = UNSET
    logo_dark: None | str | Unset = UNSET
    icon: None | str | Unset = UNSET
    brand_url: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.brand_asset_response import BrandAssetResponse # noqa: PLC0415
        from ..models.brand_colors import BrandColors # noqa: PLC0415
        from ..models.brand_dial_plan import BrandDialPlan # noqa: PLC0415
        from ..models.brand_links import BrandLinks # noqa: PLC0415
        from ..models.brand_ringtone import BrandRingtone # noqa: PLC0415
        tenant_id = self.tenant_id

        app_name = self.app_name

        colors = self.colors.to_dict()

        ringtones = []
        for ringtones_item_data in self.ringtones:
            ringtones_item = ringtones_item_data.to_dict()
            ringtones.append(ringtones_item)



        links = self.links.to_dict()

        dial_plan = self.dial_plan.to_dict()

        assets = []
        for assets_item_data in self.assets:
            assets_item = assets_item_data.to_dict()
            assets.append(assets_item)



        updated_at = self.updated_at.isoformat()

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

        brand_url: None | str | Unset
        if isinstance(self.brand_url, Unset):
            brand_url = UNSET
        else:
            brand_url = self.brand_url


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "tenant_id": tenant_id,
            "app_name": app_name,
            "colors": colors,
            "ringtones": ringtones,
            "links": links,
            "dial_plan": dial_plan,
            "assets": assets,
            "updated_at": updated_at,
        })
        if dark_colors is not UNSET:
            field_dict["dark_colors"] = dark_colors
        if logo is not UNSET:
            field_dict["logo"] = logo
        if logo_dark is not UNSET:
            field_dict["logo_dark"] = logo_dark
        if icon is not UNSET:
            field_dict["icon"] = icon
        if brand_url is not UNSET:
            field_dict["brand_url"] = brand_url

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.brand_asset_response import BrandAssetResponse # noqa: PLC0415
        from ..models.brand_colors import BrandColors # noqa: PLC0415
        from ..models.brand_dial_plan import BrandDialPlan # noqa: PLC0415
        from ..models.brand_links import BrandLinks # noqa: PLC0415
        from ..models.brand_ringtone import BrandRingtone # noqa: PLC0415
        d = dict(src_dict)
        tenant_id = d.pop("tenant_id")

        app_name = d.pop("app_name")

        colors = BrandColors.from_dict(d.pop("colors"))




        ringtones = []
        _ringtones = d.pop("ringtones")
        for ringtones_item_data in (_ringtones):
            ringtones_item = BrandRingtone.from_dict(ringtones_item_data)



            ringtones.append(ringtones_item)


        links = BrandLinks.from_dict(d.pop("links"))




        dial_plan = BrandDialPlan.from_dict(d.pop("dial_plan"))




        assets = []
        _assets = d.pop("assets")
        for assets_item_data in (_assets):
            assets_item = BrandAssetResponse.from_dict(assets_item_data)



            assets.append(assets_item)


        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




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


        def _parse_brand_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        brand_url = _parse_brand_url(d.pop("brand_url", UNSET))


        brand_response = cls(
            tenant_id=tenant_id,
            app_name=app_name,
            colors=colors,
            ringtones=ringtones,
            links=links,
            dial_plan=dial_plan,
            assets=assets,
            updated_at=updated_at,
            dark_colors=dark_colors,
            logo=logo,
            logo_dark=logo_dark,
            icon=icon,
            brand_url=brand_url,
        )


        brand_response.additional_properties = d
        return brand_response

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
